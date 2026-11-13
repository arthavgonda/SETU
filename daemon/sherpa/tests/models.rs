use setu_sherpa::bpe::SentencePiece;
use setu_sherpa::*;
use std::path::PathBuf;

fn cache() -> Option<PathBuf> {
    let p = std::env::var("SETU_TEST_MODELS")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.devcache"));
    p.is_dir().then_some(p)
}

fn s(p: PathBuf) -> String {
    p.to_string_lossy().into_owned()
}

fn resample(input: &[f32], from: u32, to: u32) -> Vec<f32> {
    let ratio = to as f64 / from as f64;
    let n = (input.len() as f64 * ratio) as usize;
    let cutoff = ratio.min(1.0);
    let half = 16i64;
    (0..n)
        .map(|i| {
            let t = i as f64 / ratio;
            let c = t.floor() as i64;
            let mut acc = 0.0;
            for k in (c - half + 1)..=(c + half) {
                if k < 0 || k as usize >= input.len() {
                    continue;
                }
                let x = t - k as f64;
                let sinc = if x.abs() < 1e-9 { 1.0 } else { (std::f64::consts::PI * x * cutoff).sin() / (std::f64::consts::PI * x) };
                let w = 0.5 + 0.5 * (std::f64::consts::PI * x / half as f64).cos();
                acc += input[k as usize] as f64 * sinc * w;
            }
            acc as f32
        })
        .collect()
}

#[test]
fn bpe_matches_reference_keywords() {
    let Some(c) = cache() else { return };
    let dir = c.join("sherpa-onnx-kws-zipformer-gigaspeech-3.3M-2024-01-01");
    let sp = SentencePiece::load(&s(dir.join("bpe.model"))).unwrap();
    let raw = std::fs::read_to_string(dir.join("keywords_raw.txt")).unwrap();
    let tok = std::fs::read_to_string(dir.join("keywords.txt")).unwrap();
    for (r, t) in raw.lines().zip(tok.lines()) {
        if r.trim().is_empty() {
            continue;
        }
        assert_eq!(sp.encode(r).join(" "), t.trim(), "{r}");
    }
}

#[test]
fn speech_round_trip() {
    let Some(c) = cache() else { return };
    let kd = c.join("sherpa-onnx-kws-zipformer-gigaspeech-3.3M-2024-01-01");
    let td = c.join("vits-piper-en_US-lessac-medium-int8");
    let tts = Tts::new(&s(td.join("en_US-lessac-medium.onnx")), &s(td.join("tokens.txt")), 2).unwrap();
    let ph = |t: &str| {
        let o = std::process::Command::new("espeak-ng").args(["-q", "--ipa", "-v", "en-us", t]).output().unwrap();
        String::from_utf8_lossy(&o.stdout).split_whitespace().collect::<Vec<_>>().join(" ")
    };
    let sentence = format!("{}, {}.", ph("Hey Setu"), ph("open Firefox"));
    let sp = SentencePiece::load(&s(kd.join("bpe.model"))).unwrap();
    let kw = format!("{} @HEY_SETU", sp.encode("HEY SETU").join(" "));
    eprintln!("keyword line: {kw}");
    let kws = Kws::new(&KwsOptions {
        model: TransducerPaths {
            encoder: s(kd.join("encoder-epoch-12-avg-2-chunk-16-left-64.int8.onnx")),
            decoder: s(kd.join("decoder-epoch-12-avg-2-chunk-16-left-64.onnx")),
            joiner: s(kd.join("joiner-epoch-12-avg-2-chunk-16-left-64.int8.onnx")),
            tokens: s(kd.join("tokens.txt")),
        },
        threads: 1,
        keywords: kw.clone(),
        score: 3.5,
        threshold: 0.10,
        trailing_blanks: 1,
    })
    .unwrap();
    let mut hit = None;
    let mut pcm = Vec::new();
    for take in 0..3 {
        let mut audio = Vec::new();
        let t0 = std::time::Instant::now();
        tts.generate(std::slice::from_ref(&sentence), 0, 1.0, &mut |chunk| {
            audio.extend_from_slice(chunk);
            true
        });
        eprintln!("take {take}: tts {} samples at {} Hz in {:?}", audio.len(), tts.sample_rate(), t0.elapsed());
        pcm = vec![0f32; 8000];
        pcm.extend(resample(&audio, tts.sample_rate(), 16_000));
        pcm.extend(vec![0f32; 16000]);
        let st = kws.stream("").unwrap();
        let t0 = std::time::Instant::now();
        for chunk in pcm.chunks(1600) {
            kws.accept(&st, chunk);
            if let Some(h) = kws.poll(&st) {
                hit.get_or_insert(h);
            }
        }
        eprintln!("take {take}: kws {:?} in {:?} for {:.2}s audio", hit, t0.elapsed(), pcm.len() as f32 / 16000.0);
        if hit.is_some() {
            break;
        }
    }
    assert!(hit.is_some_and(|h| h.keyword.contains("SETU")));

    let rd = c.join("sherpa-onnx-streaming-zipformer-en-kroko-2025-08-06");
    let asr = OnlineAsr::new(
        &TransducerPaths {
            encoder: s(rd.join("encoder.onnx")),
            decoder: s(rd.join("decoder.onnx")),
            joiner: s(rd.join("joiner.onnx")),
            tokens: s(rd.join("tokens.txt")),
        },
        2,
    )
    .unwrap();
    let os = asr.stream();
    let t0 = std::time::Instant::now();
    for chunk in pcm.chunks(1600) {
        asr.accept(&os, chunk);
        asr.decode(&os);
    }
    asr.finish(&os);
    let streaming = asr.text(&os);
    eprintln!("streaming: {streaming:?} in {:?}", t0.elapsed());
    assert!(streaming.to_lowercase().contains("firefox") || streaming.to_lowercase().contains("fire fox"));

    let pd = c.join("sherpa-onnx-nemo-parakeet_tdt_transducer_110m-en-36000-int8");
    let off = OfflineAsr::new(
        &OfflineKind::Transducer {
            paths: TransducerPaths {
                encoder: s(pd.join("encoder.int8.onnx")),
                decoder: s(pd.join("decoder.int8.onnx")),
                joiner: s(pd.join("joiner.int8.onnx")),
                tokens: s(pd.join("tokens.txt")),
            },
            nemo: true,
        },
        2,
    )
    .unwrap();
    let t0 = std::time::Instant::now();
    let r = off.recognize(&pcm);
    eprintln!("offline: {:?} in {:?}", r.text, t0.elapsed());
    assert!(r.text.to_lowercase().contains("firefox"));

    let vad = Vad::new(&VadOptions { model: s(c.join("silero_vad.onnx")), threshold: 0.5, min_silence: 0.5, min_speech: 0.25, max_speech: 20.0 }).unwrap();
    vad.accept(&pcm);
    vad.flush();
    let segs = vad.pop();
    eprintln!("vad segments: {}", segs.len());
    assert!(!segs.is_empty());

    let den = Denoiser::new(&s(c.join("gtcrn_simple.onnx"))).unwrap();
    let mut clean = Vec::new();
    for chunk in pcm.chunks(1600) {
        den.run(chunk, &mut clean);
    }
    eprintln!("denoised {} of {}", clean.len(), pcm.len());
    assert!(clean.len() > pcm.len() / 2);

    let spk = SpeakerEmbedder::new(&s(c.join("3dspeaker_speech_campplus_sv_en_voxceleb_16k.onnx"))).unwrap();
    let e = spk.embed(&pcm).unwrap();
    eprintln!("speaker embedding dim {}", e.len());
    assert!(e.len() > 100);
}
