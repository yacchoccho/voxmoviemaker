//! Linux system ALSA ABI only. No ALSA implementation or library is bundled.
//! The handle and all function calls stay on the owning audio thread.
use crate::Result;
use std::{
    ffi::{CString, c_char, c_int, c_long, c_uint, c_ulong, c_void},
    ptr,
    time::Instant,
};

type Open = unsafe extern "C" fn(*mut *mut c_void, *const c_char, c_int, c_int) -> c_int;
type Close = unsafe extern "C" fn(*mut c_void) -> c_int;
type Params =
    unsafe extern "C" fn(*mut c_void, c_int, c_int, c_uint, c_uint, c_int, c_uint) -> c_int;
type Write = unsafe extern "C" fn(*mut c_void, *const c_void, c_ulong) -> c_long;
type Delay = unsafe extern "C" fn(*mut c_void, *mut c_long) -> c_int;
type Recover = unsafe extern "C" fn(*mut c_void, c_int, c_int) -> c_int;
type Sizes = unsafe extern "C" fn(*mut c_void, *mut c_ulong, *mut c_ulong) -> c_int;

struct Library(*mut c_void);
impl Drop for Library {
    fn drop(&mut self) {
        // SAFETY: this is the unique dlopen handle; PCM was closed first.
        unsafe {
            libc::dlclose(self.0);
        }
    }
}
pub struct AudioDevice {
    pcm: *mut c_void,
    close: Close,
    drop_pcm: Close,
    write: Write,
    delay: Delay,
    recover: Recover,
    start_pcm: Close,
    state: Close,
    submitted: u64,
    base: u64,
    played: u64,
    underruns: u64,
    started: Option<Instant>,
    prefill: u64,
    _library: Library,
}
impl AudioDevice {
    pub fn open(name: &str) -> Result<Self> {
        let name = CString::new(name).map_err(|_| "audio-device-unavailable")?;
        // SAFETY: fixed library name, no downloaded or bundled code. RTLD_LOCAL.
        let handle = unsafe {
            libc::dlopen(
                c"libasound.so.2".as_ptr(),
                libc::RTLD_NOW | libc::RTLD_LOCAL,
            )
        };
        if handle.is_null() {
            return Err("audio-library-missing".into());
        }
        let library = Library(handle);
        macro_rules! symbol {
            ($name:expr, $ty:ty) => {{
                // SAFETY: names and C signatures below match the public ALSA PCM ABI.
                let pointer = unsafe { libc::dlsym(handle, $name.as_ptr()) };
                if pointer.is_null() {
                    return Err("audio-library-missing".into());
                }
                unsafe { std::mem::transmute::<*mut c_void, $ty>(pointer) }
            }};
        }
        let open = symbol!(c"snd_pcm_open", Open);
        let close = symbol!(c"snd_pcm_close", Close);
        let params = symbol!(c"snd_pcm_set_params", Params);
        let drop_pcm = symbol!(c"snd_pcm_drop", Close);
        let write = symbol!(c"snd_pcm_writei", Write);
        let delay = symbol!(c"snd_pcm_delay", Delay);
        let recover = symbol!(c"snd_pcm_recover", Recover);
        let start_pcm = symbol!(c"snd_pcm_start", Close);
        let state = symbol!(c"snd_pcm_state", Close);
        let sizes = symbol!(c"snd_pcm_get_params", Sizes);
        let mut pcm = ptr::null_mut();
        // SAFETY: ALSA allocates the PCM handle; 0=playback, 1=nonblocking.
        let status = unsafe { open(&mut pcm, name.as_ptr(), 0, 1) };
        if status < 0 {
            return Err(format!("audio-device-unavailable: {status}"));
        }
        let mut device = Self {
            pcm,
            close,
            drop_pcm,
            write,
            delay,
            recover,
            start_pcm,
            state,
            submitted: 0,
            base: 0,
            played: 0,
            underruns: 0,
            started: None,
            prefill: 0,
            _library: library,
        };
        // S16_LE=2, RW_INTERLEAVED=3, stereo/48kHz; bounded 200ms buffer.
        // Keep set_params' automatic near-full start threshold: starting with
        // only 30ms can starve a PipeWire graph before its next processing cycle.
        // SAFETY: valid PCM, enum values from ALSA's public headers.
        let status = unsafe { params(pcm, 2, 3, 2, 48_000, 1, 200_000) };
        if status < 0 {
            return Err(format!("audio-device-unavailable: {status}"));
        }
        let (mut buffer, mut period) = (0, 0);
        // SAFETY: valid configured PCM and writable frame count outputs.
        let status = unsafe { sizes(pcm, &mut buffer, &mut period) };
        if status < 0 || buffer == 0 || period == 0 {
            return Err("audio-device-unavailable".into());
        }
        device.prefill = buffer;
        Ok(device)
    }
    /// Returns frames accepted; EAGAIN leaves the caller free to poll cancellation.
    pub fn write(&mut self, bytes: &[u8]) -> Result<usize> {
        if !bytes.len().is_multiple_of(4) {
            return Err("audio-frame-invalid".into());
        }
        // Bound writes even for sinks (e.g. ALSA null) which consume instantly.
        // This also prevents an erroneous delay report from racing the clock.
        let ahead = self.submitted().saturating_sub(self.position());
        let frames = (bytes.len() / 4).min(48_000u64.saturating_sub(ahead) as usize);
        if frames == 0 {
            return Ok(0);
        }
        // SAFETY: bytes contains complete interleaved stereo S16_LE frames.
        let count = unsafe { (self.write)(self.pcm, bytes.as_ptr().cast(), frames as c_ulong) };
        if count == -(libc::EAGAIN as c_long) {
            return Ok(0);
        }
        if count == -(libc::EINTR as c_long) {
            return Ok(0);
        }
        if count < 0 {
            // Suspended/disconnected devices stop playback; avoid a blocking resume
            // loop inside snd_pcm_recover, so cancellation stays responsive.
            if count != -(libc::EPIPE as c_long) {
                return Err(format!("audio-playback-failed: {count}"));
            }
            // Preserve the last audible position when an underrun resets the buffer.
            self.position();
            // SAFETY: valid PCM; only recover the nonblocking underrun case.
            if unsafe { (self.recover)(self.pcm, count as c_int, 1) } < 0 {
                return Err(format!("audio-playback-failed: {count}"));
            }
            self.base += self.submitted;
            self.submitted = 0;
            self.underruns += 1;
            return Ok(0);
        }
        self.submitted += count as u64;
        if self.submitted >= self.prefill {
            // Some virtual PCMs don't implement ALSA's automatic threshold.
            self.start()?;
        }
        Ok(count as usize)
    }
    pub fn start(&mut self) -> Result<()> {
        // SAFETY: 2=PREPARED. A short final buffer also needs an explicit start.
        if unsafe { (self.state)(self.pcm) } == 2 && unsafe { (self.start_pcm)(self.pcm) } < 0 {
            return Err("audio-playback-failed".into());
        }
        Ok(())
    }
    /// Master clock: accepted frames minus frames still waiting in the device.
    pub fn position(&mut self) -> u64 {
        let mut delay = 0;
        // SAFETY: valid PCM and writable delay output.
        let status = unsafe { (self.delay)(self.pcm, &mut delay) };
        // SAFETY: valid PCM; 3=RUNNING. PREPARED data is never audible.
        if status >= 0 && unsafe { (self.state)(self.pcm) } == 3 {
            let played = self.base + self.submitted.saturating_sub(delay.max(0) as u64);
            if played == 0 {
                return self.played;
            }
            // Anchor to the first audible report, not snd_pcm_start: server
            // activation and output latency can postpone the first sound.
            let started = *self.started.get_or_insert_with(Instant::now);
            // A device/plugin can briefly underreport pending frames. Physical
            // playback cannot finish faster than the sample duration. Preserve
            // the audio clock's stalls, but cap advances by monotonic elapsed time.
            let elapsed = (started.elapsed().as_nanos() * 48_000 / 1_000_000_000)
                .min(u64::MAX as u128) as u64;
            self.played = self.played.max(played.min(elapsed));
        }
        self.played
    }
    pub fn submitted(&self) -> u64 {
        self.base + self.submitted
    }
    pub fn underruns(&self) -> u64 {
        self.underruns
    }
}
impl Drop for AudioDevice {
    fn drop(&mut self) {
        // SAFETY: unique valid PCM; discard pending samples before closing. Never drain on UI stop.
        unsafe {
            (self.drop_pcm)(self.pcm);
            (self.close)(self.pcm);
        }
    }
}
