use setu_sherpa::bpe::SentencePiece;
use setu_sherpa::{Kws, KwsOptions, TransducerPaths, read_wave};
use std::path::PathBuf;

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    if a.len() < 3 {
        eprintln!("usage: wake_eval MODEL_DIR KEYWORDS_FILE WAV...");
        std::process::exit(2);
    }
    let dir = PathBuf::from(&a[0]);
    let f = |n: &str| dir.join(n).to_string_lossy().into_owned();
    if let Ok(model) = std::env::var("EMBED") {
        let e = setu_sherpa::SpeakerEmbedder::new(&model).expect("speaker model");
        for w in &a[2..] {
            let Some((pcm, _)) = read_wave(w) else { continue };
            let pcm = match std::env::var("FIT").as_deref() {
                Ok("cut") => setu_sherpa::fit_speaker(&pcm, setu_sherpa::Fit::Cut),
                Ok("fill") => setu_sherpa::fit_speaker(&pcm, setu_sherpa::Fit::Fill),
                Ok(m) if m.starts_with("mixed") => setu_sherpa::fit_speaker(&pcm, setu_sherpa::Fit::Mixed(m[5..].parse::<usize>().unwrap_or(11_200))),
                _ => pcm,
            };
            let Some(v) = e.embed(&pcm) else { continue };
            let n = v.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-9);
            let unit: Vec<String> = v.iter().map(|x| format!("{:.5}", x / n)).collect();
            println!("{}\t{}\t{}", std::path::Path::new(w).file_name().unwrap().to_string_lossy(), pcm.len(), unit.join(","));
        }
        return;
    }
    if let Ok(model) = std::env::var("SPK") {
        let e = setu_sherpa::SpeakerEmbedder::new(&model).expect("speaker model");
        let unit = |v: Vec<f32>| {
            let n = v.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-9);
            v.into_iter().map(|x| x / n).collect::<Vec<f32>>()
        };
        let mut cent: Vec<f32> = Vec::new();
        let mut k = 0;
        for w in a[2..].iter().filter(|w| w.contains("/enr-")) {
            let (pcm, _) = read_wave(w).expect("wav");
            let v = unit(e.embed(&pcm).expect("embed"));
            if cent.is_empty() {
                cent = vec![0.0; v.len()];
            }
            for (c, x) in cent.iter_mut().zip(&v) {
                *c += x;
            }
            k += 1;
        }
        let cent = unit(cent.into_iter().map(|c| c / k as f32).collect());
        for w in a[2..].iter().filter(|w| !w.contains("/enr-")) {
            let (pcm, _) = read_wave(w).expect("wav");
            let v = unit(e.embed(&pcm).expect("embed"));
            let score: f32 = v.iter().zip(&cent).map(|(a, b)| a * b).sum();
            println!("{:.3}\t{}", score, std::path::Path::new(w).file_name().unwrap().to_string_lossy());
        }
        return;
    }
    if std::env::var("ASR").is_ok() {
        let kroko = dir.join("encoder.onnx").is_file();
        let asr = setu_sherpa::OnlineAsr::new(
            &if kroko {
                TransducerPaths { encoder: f("encoder.onnx"), decoder: f("decoder.onnx"), joiner: f("joiner.onnx"), tokens: f("tokens.txt") }
            } else {
                TransducerPaths {
                    encoder: f("encoder-epoch-12-avg-2-chunk-16-left-64.int8.onnx"),
                    decoder: f("decoder-epoch-12-avg-2-chunk-16-left-64.onnx"),
                    joiner: f("joiner-epoch-12-avg-2-chunk-16-left-64.int8.onnx"),
                    tokens: f("tokens.txt"),
                }
            },
            1,
        )
        .expect("asr");
        for w in &a[2..] {
            let (pcm, _) = read_wave(w).expect("wav");
            let st = asr.stream();
            let mut padded = vec![0.0f32; 4000];
            padded.extend_from_slice(&pcm);
            padded.extend(std::iter::repeat_n(0.0, 8000));
            for c in padded.chunks(1600) {
                asr.accept(&st, c);
                asr.decode(&st);
            }
            asr.finish(&st);
            println!("{}\t{}", std::path::Path::new(w).file_name().unwrap().to_string_lossy(), asr.text(&st));
        }
        return;
    }
    let sp = SentencePiece::load(&f("bpe.model")).expect("bpe");
    let mut lines = Vec::new();
    for l in std::fs::read_to_string(&a[1]).expect("keywords").lines() {
        let l = l.trim();
        if l.is_empty() {
            continue;
        }
        let (text, rest) = l.split_once('|').unwrap_or((l, ""));
        let toks = sp.encode(text.trim());
        lines.push(format!("{} {}", toks.join(" "), rest.trim()));
    }
    let kws = Kws::new(&KwsOptions {
        model: TransducerPaths {
            encoder: f("encoder-epoch-12-avg-2-chunk-16-left-64.int8.onnx"),
            decoder: f("decoder-epoch-12-avg-2-chunk-16-left-64.onnx"),
            joiner: f("joiner-epoch-12-avg-2-chunk-16-left-64.int8.onnx"),
            tokens: f("tokens.txt"),
        },
        threads: 1,
        keywords: lines[0].clone(),
        score: 3.5,
        threshold: 0.1,
        trailing_blanks: 1,
    })
    .expect("kws");
    let joined = lines.join("/");
    let (mut tp, mut fp, mut np, mut nn) = (0, 0, 0, 0);
    for w in &a[2..] {
        let (pcm, sr) = read_wave(w).expect("wav");
        assert_eq!(sr, 16_000);
        let st = kws.stream(&joined).expect("stream");
        let mut padded = vec![0.0f32; 8000];
        padded.extend_from_slice(&pcm);
        padded.extend(std::iter::repeat_n(0.0, 16000));
        let mut hits = Vec::new();
        for c in padded.chunks(1600) {
            kws.accept(&st, c);
            if let Some(h) = kws.poll(&st) {
                hits.push(h.keyword);
            }
        }
        let name = std::path::Path::new(w).file_name().unwrap().to_string_lossy().into_owned();
        let wake = hits.iter().any(|h| h.starts_with('W'));
        if std::env::var("VERBOSE").is_ok() {
            println!("{name} {hits:?}");
        }
        if name.starts_with("pos") {
            np += 1;
            if wake {
                tp += 1;
            } else {
                println!("MISS {name} {hits:?}");
            }
        } else {
            nn += 1;
            if wake {
                fp += 1;
                println!("FALSE {name} {hits:?}");
            }
        }
    }
    println!("recall {tp}/{np}  false {fp}/{nn}");
}
