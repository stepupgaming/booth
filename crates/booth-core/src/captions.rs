//! Caption files for an exported voiceover: sentence captions (two rows at most) or short ones
//! of 2 to 4 words for Shorts, TikTok and Reels, as SubRip (.srt) and WebVTT (.vtt).

use serde::{Deserialize, Serialize};

use crate::script::{self, Cue};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Style {
    /// A sentence or a clause at a time, up to two rows.
    #[default]
    Sentences,
    /// 2 to 4 words at a time, one row.
    Short,
}

/// Longest short caption, in characters, before it ends early (at two words or more).
const SHORT_MAX: usize = 22;

fn ends_phrase(word: &str) -> bool {
    word.trim_end_matches(['"', '\'', '”', '’', ')']).ends_with(['.', ',', '!', '?', ';', ':', '—', '–', '…', '。', '！', '？', '，'])
}

/// A line's words in groups of 2 to 4: a group ends at 4 words, at punctuation, or when it gets
/// long. A line of one word is one group; a lone word at the end joins the group before it.
pub fn short_chunks(text: &str) -> Vec<String> {
    let mut groups: Vec<Vec<&str>> = Vec::new();
    let mut cur: Vec<&str> = Vec::new();
    for w in text.split_whitespace() {
        cur.push(w);
        let chars = cur.iter().map(|w| w.chars().count()).sum::<usize>() + cur.len() - 1;
        if cur.len() >= 4 || (cur.len() >= 2 && (ends_phrase(w) || chars >= SHORT_MAX)) {
            groups.push(std::mem::take(&mut cur));
        }
    }
    if !cur.is_empty() {
        match groups.last_mut() {
            Some(last) if cur.len() == 1 && last.len() < 4 => last.append(&mut cur),
            Some(last) if cur.len() == 1 => {
                let w = last.pop().unwrap_or_default();
                cur.insert(0, w);
                groups.push(cur);
            }
            _ => groups.push(cur),
        }
    }
    groups.into_iter().map(|g| g.join(" ")).collect()
}

/// Captions for spoken lines: each line's text, where it starts in the voiceover and how long
/// it lasts (from the line's audio). Within a line, captions are timed by their characters.
pub fn cues(lines: &[(String, f64, f64)], style: Style) -> Vec<Cue> {
    match style {
        Style::Sentences => script::cues(lines),
        Style::Short => {
            let mut out = Vec::new();
            for (text, start, secs) in lines {
                let pieces = short_chunks(text);
                let total: usize = pieces.iter().map(|p| p.chars().count().max(1)).sum();
                let mut t = *start;
                for p in pieces {
                    let d = secs * p.chars().count().max(1) as f64 / total.max(1) as f64;
                    out.push(Cue { start: t, end: t + d, text: p });
                    t += d;
                }
            }
            out
        }
    }
}

fn time(t: f64, sep: char) -> String {
    let ms = (t.max(0.0) * 1000.0).round() as u64;
    format!("{:02}:{:02}:{:02}{sep}{:03}", ms / 3_600_000, ms / 60_000 % 60, ms / 1000 % 60, ms % 1000)
}

/// SubRip (.srt): numbered cues, `00:00:01,600` times.
pub fn to_srt(cues: &[Cue]) -> String {
    let mut s = String::new();
    for (i, c) in cues.iter().enumerate() {
        s.push_str(&format!("{}\n{} --> {}\n{}\n\n", i + 1, time(c.start, ','), time(c.end, ','), c.text));
    }
    s
}

/// WebVTT (.vtt): a `WEBVTT` header, `00:00:01.600` times.
pub fn to_vtt(cues: &[Cue]) -> String {
    let mut s = String::from("WEBVTT\n\n");
    for c in cues {
        s.push_str(&format!("{} --> {}\n{}\n\n", time(c.start, '.'), time(c.end, '.'), c.text));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_chunks_are_two_to_four_words() {
        let c = short_chunks("You walk outside, it's minus ten, and your phone drops from 40 percent to zero.");
        assert_eq!(c, ["You walk outside,", "it's minus ten,", "and your phone drops", "from 40 percent", "to zero."]);
        // a lone word at the end takes a word from a full group, or joins a shorter one
        assert_eq!(short_chunks("It isn't broken at all."), ["It isn't broken", "at all."]);
        assert_eq!(short_chunks("one two three four five"), ["one two three", "four five"]);
        assert_eq!(short_chunks("Go on, then."), ["Go on, then."]);
        assert_eq!(short_chunks("Wait."), ["Wait."]);
        assert_eq!(short_chunks("Yes. No."), ["Yes. No."]);
        assert!(short_chunks("  ").is_empty());
        // long words end a group early, but never below two words
        assert_eq!(
            short_chunks("internationalization considerations everywhere today"),
            ["internationalization considerations", "everywhere today"]
        );
        for line in ["The battery is just slow, and that's all there is to it, honestly, truly.", "a b c d e f g h i j k"] {
            let c = short_chunks(line);
            assert!(c.iter().all(|g| (2..=4).contains(&g.split_whitespace().count())), "{c:?}");
            assert_eq!(c.join(" "), line);
        }
    }

    #[test]
    fn short_cues_fill_each_line_exactly() {
        let c = cues(&[("It isn't broken. The battery is just slow.".into(), 5.45, 2.6), ("Wait.".into(), 8.3, 0.5)], Style::Short);
        let texts: Vec<&str> = c.iter().map(|c| c.text.as_str()).collect();
        assert_eq!(texts, ["It isn't broken.", "The battery is", "just slow.", "Wait."]);
        // timed by characters inside the line's own start and end
        assert!((c[0].start - 5.45).abs() < 1e-9 && (c[2].end - 8.05).abs() < 1e-9);
        let chars = [16.0, 14.0, 10.0];
        assert!((c[1].end - c[1].start - 2.6 * chars[1] / 40.0).abs() < 1e-9);
        assert!((c[3].start - 8.3).abs() < 1e-9 && (c[3].end - 8.8).abs() < 1e-9);
        assert!(c.windows(2).all(|w| w[0].end <= w[1].start + 1e-9));
        // sentence style is the captions Booth always made
        let lines = [("It isn't broken.".to_string(), 0.0, 1.0)];
        assert_eq!(cues(&lines, Style::Sentences), script::cues(&lines));
    }

    #[test]
    fn srt_and_vtt_times() {
        let c = vec![
            Cue { start: 0.0, end: 1.6, text: "First".into() },
            Cue { start: 3725.0405, end: 3726.9996, text: "Two\nrows".into() },
        ];
        assert_eq!(to_srt(&c), "1\n00:00:00,000 --> 00:00:01,600\nFirst\n\n2\n01:02:05,041 --> 01:02:07,000\nTwo\nrows\n\n");
        assert_eq!(to_vtt(&c), "WEBVTT\n\n00:00:00.000 --> 00:00:01.600\nFirst\n\n01:02:05.041 --> 01:02:07.000\nTwo\nrows\n\n");
        assert_eq!(to_srt(&c), script::to_srt(&c));
    }
}
