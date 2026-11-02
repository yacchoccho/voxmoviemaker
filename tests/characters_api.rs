use std::{
    io::{Read, Write},
    net::TcpListener,
    thread,
    time::{Duration, Instant},
};
use voxmoviemaker::{project::VoicevoxSettings, voicevox::Client};

fn wav() -> Vec<u8> {
    let mut bytes = std::io::Cursor::new(Vec::new());
    let mut writer = hound::WavWriter::new(
        &mut bytes,
        hound::WavSpec {
            channels: 2,
            sample_rate: 24000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        },
    )
    .unwrap();
    for _ in 0..4800 {
        writer.write_sample(100i16).unwrap();
    }
    writer.finalize().unwrap();
    bytes.into_inner()
}
type Requests = Vec<(String, Vec<u8>)>;
fn server(replies: Vec<Vec<u8>>) -> (Client, thread::JoinHandle<Requests>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let client = Client::new(&format!("http://{}", listener.local_addr().unwrap()), false).unwrap();
    let handle = thread::spawn(move || {
        let mut requests = Vec::new();
        for reply in replies {
            let deadline = Instant::now() + Duration::from_secs(4);
            let mut stream = loop {
                if let Ok((stream, _)) = listener.accept() {
                    break stream;
                }
                assert!(Instant::now() < deadline, "missing API request");
                thread::sleep(Duration::from_millis(2));
            };
            stream
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut header = Vec::new();
            let mut byte = [0];
            while !header.ends_with(b"\r\n\r\n") {
                stream.read_exact(&mut byte).unwrap();
                header.push(byte[0]);
                assert!(header.len() < 16384);
            }
            let header = String::from_utf8(header).unwrap();
            let len = header
                .lines()
                .find_map(|line| {
                    line.to_ascii_lowercase()
                        .strip_prefix("content-length:")
                        .and_then(|s| s.trim().parse::<usize>().ok())
                })
                .unwrap_or(0);
            let mut body = vec![0; len];
            stream.read_exact(&mut body).unwrap();
            requests.push((header, body));
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                reply.len()
            )
            .unwrap();
            stream.write_all(&reply).unwrap();
        }
        requests
    });
    (client, handle)
}

#[test]
fn advanced_query_values_and_style_id_are_sent_to_the_correct_api_locations() {
    let (client,server) = server(vec![
        br#"{"accent_phrases":[],"pauseLength":null,"pauseLengthScale":1}"#.to_vec(),
        br#"{"paths":{"/synthesis":{"post":{"parameters":[{"name":"enable_interrogative_upspeak","in":"query"}]}}}}"#.to_vec(),
        wav(),
    ]);
    let settings = VoicevoxSettings {
        speaker: 42,
        speaker_uuid: "not-an-api-argument".into(),
        speed: 1.4,
        pitch: 0.0,
        intonation: 1.2,
        volume: 0.0,
        pre_phoneme: 0.0,
        post_phoneme: 0.2,
        pause_length: Some(0.0),
        pause_scale: Some(1.6),
        sampling_rate: 24000,
        stereo: true,
        interrogative_upspeak: false,
        ..Default::default()
    };
    let (_, duration, rate) = client.synthesize("テストです。", &settings).unwrap();
    assert_eq!(rate, 24000);
    assert!((duration - 0.1).abs() < 0.001);
    let requests = server.join().unwrap();
    assert!(requests[0].0.starts_with("POST /audio_query?"));
    assert!(requests[0].0.contains("speaker=42"));
    assert!(requests[1].0.starts_with("GET /openapi.json"));
    assert!(requests[2].0.contains("speaker=42"));
    assert!(requests[2].0.contains("enable_interrogative_upspeak=false"));
    let body: serde_json::Value = serde_json::from_slice(&requests[2].1).unwrap();
    for (key, expected) in [
        ("speedScale", 1.4),
        ("pitchScale", 0.0),
        ("intonationScale", 1.2),
        ("volumeScale", 0.0),
        ("prePhonemeLength", 0.0),
        ("postPhonemeLength", 0.2),
        ("pauseLength", 0.0),
        ("pauseLengthScale", 1.6),
    ] {
        assert_eq!(body[key], expected);
    }
    assert_eq!(body["outputSamplingRate"], 24000);
    assert_eq!(body["outputStereo"], true);
    assert!(body.get("enable_interrogative_upspeak").is_none());
    assert!(
        !String::from_utf8(requests[2].1.clone())
            .unwrap()
            .contains("not-an-api-argument")
    );
}

#[test]
fn unsupported_pause_option_fails_before_synthesis_and_defaults_still_work() {
    let (client, server) = server(vec![br#"{"accent_phrases":[]}"#.to_vec()]);
    let settings = VoicevoxSettings {
        pause_length: Some(0.5),
        ..Default::default()
    };
    assert!(
        client
            .synthesize("test", &settings)
            .unwrap_err()
            .starts_with("unsupported-voice-property")
    );
    assert_eq!(server.join().unwrap().len(), 1);
    let (client, server) = self::server(vec![br#"{"accent_phrases":[]}"#.to_vec(), wav()]);
    client
        .synthesize("test", &VoicevoxSettings::default())
        .unwrap();
    assert_eq!(server.join().unwrap().len(), 2);
}

#[test]
fn unsupported_upspeak_parameter_fails_without_sending_a_synthesis_request() {
    let (client, server) = server(vec![
        br#"{"accent_phrases":[]}"#.to_vec(),
        br#"{"paths":{"/synthesis":{"post":{"parameters":[]}}}}"#.to_vec(),
    ]);
    let settings = VoicevoxSettings {
        interrogative_upspeak: false,
        ..Default::default()
    };
    assert!(
        client
            .synthesize("test", &settings)
            .unwrap_err()
            .starts_with("unsupported-voice-property")
    );
    assert_eq!(server.join().unwrap().len(), 2);
}

#[test]
fn invalid_advanced_values_are_rejected_before_network_access() {
    let client = Client::new("http://127.0.0.1:1", false).unwrap();
    for settings in [
        VoicevoxSettings {
            pause_length: Some(-1.0),
            ..Default::default()
        },
        VoicevoxSettings {
            pause_scale: Some(f64::NAN),
            ..Default::default()
        },
        VoicevoxSettings {
            sampling_rate: 0,
            ..Default::default()
        },
        VoicevoxSettings {
            speed: 0.0,
            ..Default::default()
        },
    ] {
        assert_eq!(
            client.synthesize("test", &settings).unwrap_err(),
            "invalid-voice-settings"
        );
    }
}
