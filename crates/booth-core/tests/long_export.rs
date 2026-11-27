//! A long script exports without holding the voiceover in memory: the most memory in use during
//! an export grows with the script's text, not with its audio.

use std::alloc::{GlobalAlloc, Layout, System};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};

use booth_core::audio::Format;
use booth_core::batch::{self, ExportOptions, Job, JobLine, LineState};
use booth_core::captions::Style;

struct Counting;

static NOW: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        let p = unsafe { System.alloc(l) };
        if !p.is_null() {
            let now = NOW.fetch_add(l.size(), Ordering::Relaxed) + l.size();
            PEAK.fetch_max(now, Ordering::Relaxed);
        }
        p
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        unsafe { System.dealloc(p, l) };
        NOW.fetch_sub(l.size(), Ordering::Relaxed);
    }
}

#[global_allocator]
static A: Counting = Counting;

fn sine(path: &std::path::Path, secs: f64, amp: f32) {
    // Booth's engine speaks at 44.1 kHz mono.
    let spec = hound::WavSpec { channels: 1, sample_rate: 44_100, bits_per_sample: 16, sample_format: hound::SampleFormat::Int };
    let mut w = hound::WavWriter::create(path, spec).unwrap();
    for i in 0..(secs * 44_100.0) as usize {
        w.write_sample((amp * (i as f32 * 0.07).sin() * 32767.0) as i16).unwrap();
    }
    w.finalize().unwrap();
}

/// A made script of `n` one-second lines; returns its id.
fn job(n: usize) -> String {
    let id = format!("long-{n}");
    let folder = batch::scripts_dir().join(&id);
    std::fs::create_dir_all(&folder).unwrap();
    let lines = (0..n)
        .map(|i| {
            let path = folder.join(format!("line-{:03}-v1.wav", i + 1));
            sine(&path, 1.0, if i % 3 == 0 { 0.1 } else { 0.5 });
            JobLine {
                speaker: None,
                voice: "v".into(),
                voice_name: "Rupert".into(),
                text: format!("Line number {i} says a few words here."),
                para: i % 10 == 0,
                pause_ms: 0,
                state: LineState::Done,
                seconds: 1.0,
                seed: None,
                version: 1,
                voice_match: None,
                drifted: false,
                cut_off: false,
                error: None,
                path: Some(path),
            }
        })
        .collect();
    let job = Job {
        id: id.clone(),
        title: format!("Long {n}"),
        created_at: chrono::Utc::now(),
        lines,
        skipped: Vec::new(),
        source: String::new(),
        voices: BTreeMap::new(),
        via: "app".into(),
        running: false,
        exported: None,
    };
    std::fs::write(folder.join("script.json"), serde_json::to_vec(&job).unwrap()).unwrap();
    id
}

/// Most memory in use (above what was in use before) while exporting.
fn peak_export(id: &str, o: &ExportOptions) -> (usize, f64) {
    let before = NOW.load(Ordering::Relaxed);
    PEAK.store(before, Ordering::Relaxed);
    let out = batch::export(id, o).unwrap();
    (PEAK.load(Ordering::Relaxed) - before, out.seconds)
}

#[test]
fn memory_does_not_grow_with_the_script() {
    let home = std::env::temp_dir().join(format!("booth-long-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::env::set_var("BOOTH_HOME", &home);
    std::env::set_var("BOOTH_TAKES_DIR", home.join("takes"));
    let short = job(60);
    let long = job(600);
    for o in [
        ExportOptions { lines: true, ..Default::default() },
        ExportOptions { format: Format::Mp3, even_loudness: true, caption_style: Style::Short, vtt: true, ..Default::default() },
    ] {
        let (small, secs_small) = peak_export(&short, &o);
        let (big, secs_big) = peak_export(&long, &o);
        eprintln!(
            "{:?} even={}: 60 lines ({secs_small:.0} s) peak {:.2} MB, 600 lines ({secs_big:.0} s) peak {:.2} MB",
            o.format,
            o.even_loudness,
            small as f64 / 1e6,
            big as f64 / 1e6
        );
        assert!(secs_big > 700.0);
        // Joined in memory, 600 lines would hold 68 MB of samples (88 kB a second), and the old
        // export held every line's file too. What grows here is the script itself (its text and
        // file names, a few kB a line), not the audio.
        assert!(big < 8_000_000, "{big}");
        let audio = (secs_big - secs_small) * 88_200.0;
        assert!(((big - small) as f64) < audio / 10.0, "{small} -> {big}");
    }
    let _ = std::fs::remove_dir_all(&home);
}
