//! Scripts: a whole pasted script split into the lines Booth speaks, and captions timed to the
//! finished audio.
//!
//! People paste scripts from Google Docs, ChatGPT, Notion or a notes app, so the parser takes
//! what those produce: speaker labels (`NARRATOR:`, `**Host:**`), headings, stage notes
//! (`[B-roll: ...]`, `(SFX: wind)`), bullets, blank lines between paragraphs, and long
//! paragraphs that need splitting into lines the engine reads well.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// The longest line Booth sends to the engine in one go. Longer paragraphs are split at
/// sentence ends (then at commas, then between words).
pub const MAX_LINE: usize = 180;
/// The most characters on one caption row, and rows per caption (the usual subtitle limits).
const CAPTION_ROW: usize = 42;
const CAPTION_ROWS: usize = 2;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Line {
    /// The speaker label from the script (`NARRATOR`), if the line had one.
    #[serde(default)]
    pub speaker: Option<String>,
    /// What gets said.
    pub text: String,
    /// The line starts a new paragraph, so it gets a longer pause before it.
    #[serde(default)]
    pub para: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Parsed {
    pub lines: Vec<Line>,
    /// Headings and notes that aren't spoken, in the order they appeared.
    pub skipped: Vec<String>,
    /// Speakers in order of first appearance, with how many lines each has.
    pub speakers: Vec<(String, usize)>,
}

/// Labels that mark a note for the editor rather than words to say.
const NOTE_LABELS: &[&str] = &[
    "sfx", "sound", "music", "b-roll", "broll", "b roll", "visual", "visuals", "scene", "cut to", "shot",
    "on screen", "on-screen", "text on screen", "overlay", "title", "note", "notes", "caption", "graphic",
    "graphics", "transition", "camera", "footage", "image", "thumbnail", "cta", "int.", "ext.",
];

/// Split a pasted script into spoken lines. `voice_names` lets `Rupert: hello` count as a speaker
/// label even when the script uses that name only once.
pub fn parse(text: &str, voice_names: &[String]) -> Parsed {
    let text = text.replace("\r\n", "\n").replace('\r', "\n");

    // First pass: how often each label candidate appears, so "Note: ..." used once in prose
    // isn't taken for a speaker while "Host:" on every line is.
    let mut label_count: BTreeMap<String, usize> = BTreeMap::new();
    for raw in text.lines() {
        if let Some((label, _)) = split_label(&clean_markup(raw.trim())) {
            *label_count.entry(label.to_lowercase()).or_default() += 1;
        }
    }
    let known: Vec<String> = voice_names.iter().map(|v| v.to_lowercase()).collect();
    // Once a script clearly uses speaker labels, a short name heard once ("Guest:") is one too.
    let uses_labels = label_count.iter().any(|(l, n)| *n >= 2 && !is_note_label(l));
    let is_speaker = |label: &str| {
        let low = label.to_lowercase();
        let caps = label.chars().any(char::is_alphabetic) && !label.chars().any(char::is_lowercase);
        let short_name = label.split_whitespace().count() <= 2 && label.chars().all(|c| c.is_alphabetic() || c == ' ');
        caps || label_count.get(&low).copied().unwrap_or(0) >= 2 || known.contains(&low) || uses_labels && short_name
    };

    let mut out = Parsed::default();
    // Units are one speaker's run of text, before splitting into engine-sized lines.
    let mut units: Vec<Line> = Vec::new();
    let mut blank = false;
    for raw in text.lines() {
        let mut s = clean_markup(raw.trim());
        if s.is_empty() || is_rule(&s) {
            blank = true;
            continue;
        }
        if s.starts_with('#') {
            out.skipped.push(s.clone());
            blank = true;
            continue;
        }
        if is_whole_note(&s) {
            out.skipped.push(s);
            continue;
        }
        let mut speaker = None;
        if let Some((label, rest)) = split_label(&s) {
            if is_note_label(&label) {
                out.skipped.push(s);
                continue;
            }
            if is_speaker(&label) {
                speaker = Some(label);
                s = rest;
            }
        }
        let (spoken, notes) = strip_inline_notes(&s);
        out.skipped.extend(notes);
        if spoken.is_empty() {
            continue;
        }
        // A sentence broken across lines (text copied from a PDF or an email): join it back,
        // as long as nothing marks a new speaker or paragraph.
        if let Some(prev) = units.last_mut() {
            let continues = !blank
                && speaker.is_none()
                && !ends_sentence(&prev.text)
                && spoken.chars().next().is_some_and(char::is_lowercase);
            if continues {
                prev.text.push(' ');
                prev.text.push_str(&spoken);
                continue;
            }
        }
        // A line without a label in a script that uses labels keeps the last speaker.
        let speaker = speaker.or_else(|| if blank { None } else { units.last().and_then(|u| u.speaker.clone()) });
        units.push(Line { speaker, text: spoken, para: blank && !units.is_empty() });
        blank = false;
    }

    for u in units {
        for (i, chunk) in split_long(&u.text, MAX_LINE).into_iter().enumerate() {
            out.lines.push(Line { speaker: u.speaker.clone(), text: chunk, para: u.para && i == 0 });
        }
    }
    for l in &out.lines {
        let name = l.speaker.clone().unwrap_or_default();
        match out.speakers.iter_mut().find(|(n, _)| *n == name) {
            Some((_, n)) => *n += 1,
            None => out.speakers.push((name, 1)),
        }
    }
    out
}

/// Markdown and chat formatting that shouldn't be read out: bold, italics, bullets, quotes.
fn clean_markup(s: &str) -> String {
    let mut s = s.replace("**", "").replace("__", "");
    for bullet in ["- ", "* ", "• ", "> ", "– ", "— "] {
        if let Some(rest) = s.strip_prefix(bullet) {
            s = rest.trim_start().to_string();
            break;
        }
    }
    // Single *asterisks* around words are emphasis, never speech.
    s.replace('*', "").trim().to_string()
}

fn is_rule(s: &str) -> bool {
    s.len() >= 3 && s.chars().all(|c| matches!(c, '-' | '=' | '_' | '*' | '~'))
}

/// `[B-roll: ...]`, `(SFX: wind)`, `{music}`, or a bare timestamp like `0:15` or `00:12 - 00:18`.
fn is_whole_note(s: &str) -> bool {
    let wrapped = |o: char, c: char| s.starts_with(o) && s.ends_with(c) && !s[1..s.len() - c.len_utf8()].contains(c);
    if wrapped('[', ']') || wrapped('(', ')') || wrapped('{', '}') {
        return true;
    }
    !s.is_empty() && s.chars().all(|c| c.is_ascii_digit() || matches!(c, ':' | '.' | ',' | '-' | '–' | ' ')) && s.contains(':')
}

fn is_note_label(label: &str) -> bool {
    let low = label.to_lowercase();
    NOTE_LABELS.iter().any(|n| low == *n || low.starts_with(&format!("{n} ")))
}

/// `NAME: text`, `Name (V.O.): text` or `NAME - text` is out of scope; only a colon marks a label.
/// The label is at most four words with no sentence punctuation, so "Here's the thing: ..." and
/// "Step 2: ..." stay speech unless they repeat (see `parse`).
fn split_label(s: &str) -> Option<(String, String)> {
    let colon = s.find(':')?;
    let (head, rest) = (&s[..colon], s[colon + 1..].trim());
    if rest.is_empty() || !s[colon + 1..].starts_with(char::is_whitespace) {
        return None; // "10:30" or a heading with nothing after it
    }
    // Drop a delivery note after the name: "NARRATOR (V.O.)" -> "NARRATOR".
    let name = match head.find('(') {
        Some(p) if head.trim_end().ends_with(')') => head[..p].trim(),
        _ => head.trim(),
    };
    let words = name.split_whitespace().count();
    let ok = (1..=4).contains(&words)
        && name.chars().count() <= 30
        && name.chars().next().is_some_and(char::is_alphabetic)
        && !name.chars().any(|c| matches!(c, '.' | ',' | '!' | '?' | '"' | '“' | '”' | '[' | ']' | '(' | ')'))
        || is_note_label(name);
    ok.then(|| (name.to_string(), rest.to_string()))
}

/// Takes out `[notes]`, `{notes}` and `(SFX ...)`-style notes inside a line. Ordinary
/// parentheses are speech ("(and that's fine)"), so only ones that start like a note go.
fn strip_inline_notes(s: &str) -> (String, Vec<String>) {
    let mut out = String::new();
    let mut notes = Vec::new();
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let close = match c {
            '[' => Some(']'),
            '{' => Some('}'),
            '(' => Some(')'),
            _ => None,
        };
        if let Some(close) = close {
            if let Some(len) = chars[i + 1..].iter().position(|&x| x == close) {
                let inner: String = chars[i + 1..i + 1 + len].iter().collect();
                let note = c != '(' || {
                    let low = inner.trim().to_lowercase();
                    NOTE_LABELS.iter().any(|n| low.starts_with(n))
                        || ["pause", "beat", "laughs", "sighs", "whisper", "beat"].iter().any(|n| low.starts_with(n))
                };
                if note {
                    notes.push(chars[i..=i + 1 + len].iter().collect());
                    i += len + 2;
                    continue;
                }
            }
        }
        out.push(c);
        i += 1;
    }
    let spoken = out.split_whitespace().collect::<Vec<_>>().join(" ");
    // A note removed before punctuation leaves "word ." behind.
    let spoken = [" .", " ,", " !", " ?", " ;", " :"]
        .iter()
        .fold(spoken, |acc, p| acc.replace(p, &p[1..]));
    (spoken.trim().to_string(), notes)
}

fn is_sentence_end(c: char) -> bool {
    matches!(c, '.' | '!' | '?' | '…' | '。' | '！' | '？' | '।' | '؟')
}

/// Ends like a sentence (closing quotes and brackets after the stop are fine).
fn ends_sentence(s: &str) -> bool {
    s.trim_end()
        .trim_end_matches(['"', '\'', '”', '’', ')', ']', '»'])
        .chars()
        .last()
        .is_some_and(|c| is_sentence_end(c) || c == ':' || c == ';')
}

/// Sentences, each with its trailing punctuation and closing quotes.
fn sentences(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        cur.push(chars[i]);
        if is_sentence_end(chars[i]) {
            // keep "..." and "?!" together, and closing quotes
            while i + 1 < chars.len() && (is_sentence_end(chars[i + 1]) || matches!(chars[i + 1], '"' | '\'' | '”' | '’' | ')' | '»')) {
                i += 1;
                cur.push(chars[i]);
            }
            let next = chars.get(i + 1);
            // A stop followed by a space (or the end, or CJK text that has no spaces) ends it,
            // but not "Dr. Smith" or "3.5": those have no space or a short lowercase-free word.
            let cjk = matches!(chars[i], '。' | '！' | '？');
            if cjk || next.is_none_or(|c| c.is_whitespace()) && !is_abbreviation(&cur) {
                out.push(cur.trim().to_string());
                cur.clear();
            }
        }
        i += 1;
    }
    if !cur.trim().is_empty() {
        out.push(cur.trim().to_string());
    }
    out
}

fn is_abbreviation(s: &str) -> bool {
    let last = s.trim_end_matches('.').rsplit(char::is_whitespace).next().unwrap_or("").to_lowercase();
    ["mr", "mrs", "ms", "dr", "st", "vs", "etc", "e.g", "i.e", "approx", "no", "jr", "sr", "prof", "inc", "ltd"]
        .contains(&last.as_str())
}

/// Split text into pieces of at most `max` characters: whole sentences where possible, then
/// clauses at commas, then between words.
pub fn split_long(text: &str, max: usize) -> Vec<String> {
    let len = |s: &str| s.chars().count();
    if len(text) <= max {
        return vec![text.to_string()];
    }
    let mut pieces = Vec::new();
    for s in sentences(text) {
        if len(&s) <= max {
            pieces.push(s);
            continue;
        }
        // A very long sentence: break after commas, semicolons or dashes, then between words.
        let mut clause = String::new();
        for word in s.split_whitespace() {
            if !clause.is_empty() && len(&clause) + 1 + len(word) > max {
                pieces.push(std::mem::take(&mut clause));
            }
            if !clause.is_empty() {
                clause.push(' ');
            }
            clause.push_str(word);
            let breaks = word.ends_with([',', ';', '—', '–', ':']);
            if breaks && len(&clause) >= max / 2 {
                pieces.push(std::mem::take(&mut clause));
            }
        }
        if !clause.is_empty() {
            pieces.push(clause);
        }
    }
    // Put short pieces back together up to `max`, so a paragraph doesn't become ten tiny lines.
    let mut out: Vec<String> = Vec::new();
    for p in pieces {
        match out.last_mut() {
            Some(last) if len(last) + 1 + len(&p) <= max => {
                last.push(' ');
                last.push_str(&p);
            }
            _ => out.push(p),
        }
    }
    out
}

/// One caption on screen.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Cue {
    pub start: f64,
    pub end: f64,
    pub text: String,
}

/// Captions for spoken lines, given each line's text and where it sits in the joined audio
/// (`start`, `seconds`). A long line becomes several captions, timed by how many characters
/// each one has; each caption is at most two rows.
pub fn cues(lines: &[(String, f64, f64)]) -> Vec<Cue> {
    let mut out = Vec::new();
    for (text, start, secs) in lines {
        let pieces = split_long(text, CAPTION_ROW * CAPTION_ROWS - 4);
        let total: usize = pieces.iter().map(|p| p.chars().count().max(1)).sum();
        let mut t = *start;
        for p in pieces {
            let d = secs * p.chars().count().max(1) as f64 / total as f64;
            out.push(Cue { start: t, end: t + d, text: wrap_rows(&p) });
            t += d;
        }
    }
    out
}

/// Two balanced rows when the caption is longer than one row.
fn wrap_rows(text: &str) -> String {
    let n = text.chars().count();
    if n <= CAPTION_ROW {
        return text.to_string();
    }
    // The space that makes the longer row shortest, so both rows are about as long.
    let best = text
        .char_indices()
        .filter(|(_, c)| *c == ' ')
        .map(|(b, _)| (b, text[..b].chars().count()))
        .min_by_key(|(_, left)| (*left).max(n - left - 1));
    match best {
        Some((b, _)) => format!("{}\n{}", &text[..b], &text[b + 1..]),
        None => text.to_string(),
    }
}

fn srt_time(t: f64) -> String {
    let ms = (t.max(0.0) * 1000.0).round() as u64;
    format!("{:02}:{:02}:{:02},{:03}", ms / 3_600_000, ms / 60_000 % 60, ms / 1000 % 60, ms % 1000)
}

/// SubRip (.srt), which YouTube, CapCut, Premiere, Resolve and every editor read.
pub fn to_srt(cues: &[Cue]) -> String {
    let mut s = String::new();
    for (i, c) in cues.iter().enumerate() {
        s.push_str(&format!("{}\n{} --> {}\n{}\n\n", i + 1, srt_time(c.start), srt_time(c.end), c.text));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(p: &Parsed) -> Vec<&str> {
        p.lines.iter().map(|l| l.text.as_str()).collect()
    }

    #[test]
    fn faceless_script() {
        let src = "# Why your phone dies in the cold\n\n[B-roll: phone screen at 1%, snowy street]\n\n\
NARRATOR: You walk outside, it's minus ten, and your phone drops from 40 percent to zero.\n\
NARRATOR: It isn't broken. The battery is just slow.\n\n(SFX: wind)\n\n\
NARRATOR: Inside every lithium-ion battery, ions move between two sides.\n\n\
QUOTE: Warm it up, and most of that charge comes back.\n";
        let p = parse(src, &[]);
        assert_eq!(
            texts(&p),
            [
                "You walk outside, it's minus ten, and your phone drops from 40 percent to zero.",
                "It isn't broken. The battery is just slow.",
                "Inside every lithium-ion battery, ions move between two sides.",
                "Warm it up, and most of that charge comes back.",
            ]
        );
        assert_eq!(p.lines[0].speaker.as_deref(), Some("NARRATOR"));
        assert_eq!(p.lines[3].speaker.as_deref(), Some("QUOTE"));
        assert_eq!(p.lines.iter().map(|l| l.para).collect::<Vec<_>>(), [false, false, true, true]);
        assert_eq!(p.skipped, ["# Why your phone dies in the cold", "[B-roll: phone screen at 1%, snowy street]", "(SFX: wind)"]);
        assert_eq!(p.speakers, [("NARRATOR".to_string(), 3), ("QUOTE".to_string(), 1)]);
    }

    #[test]
    fn plain_prose_and_labels_that_are_speech() {
        let p = parse("Here's the thing: nobody tells you this.\nStep 2: open the app.", &[]);
        assert_eq!(texts(&p), ["Here's the thing: nobody tells you this.", "Step 2: open the app."]);
        assert!(p.lines.iter().all(|l| l.speaker.is_none()));
        // ...but a label that repeats is a speaker, and a known voice name counts at once
        let p = parse("Host: Welcome back.\nGuest: Thanks.\nHost: So.", &[]);
        assert_eq!(p.lines[1].speaker.as_deref(), Some("Guest"));
        let p = parse("Rupert: Hello there.", &["Rupert".into()]);
        assert_eq!(p.lines[0].speaker.as_deref(), Some("Rupert"));
    }

    #[test]
    fn markdown_notes_and_broken_lines() {
        let p = parse("**NARRATOR (V.O.):** Then it *stopped*. [cut to black]\n- and then\nwe left (pause) quietly.", &[]);
        assert_eq!(texts(&p), ["Then it stopped.", "and then we left quietly."]);
        assert_eq!(p.lines[0].speaker.as_deref(), Some("NARRATOR"));
        assert_eq!(p.skipped, ["[cut to black]", "(pause)"]);
        let p = parse("SFX: whoosh\nMusic: low drone\n0:15\nReal line (and that's fine).", &[]);
        assert_eq!(texts(&p), ["Real line (and that's fine)."]);
    }

    #[test]
    fn long_paragraphs_split_at_sentences() {
        let para = "Dr. Smith said it twice. ".repeat(4) + &"word ".repeat(60);
        let parts = split_long(para.trim(), 120);
        assert!(parts.iter().all(|p| p.chars().count() <= 120), "{parts:?}");
        assert!(parts[0].starts_with("Dr. Smith said it twice. Dr. Smith"));
        assert_eq!(parts.join(" ").split_whitespace().count(), para.split_whitespace().count());
        assert_eq!(sentences("Wait... what?! Yes. 3.5 percent."), ["Wait...", "what?!", "Yes.", "3.5 percent."]);
        assert_eq!(sentences("你好。今天很冷！"), ["你好。", "今天很冷！"]);
    }

    #[test]
    fn captions() {
        let c = cues(&[
            ("You walk outside, it's minus ten, and your phone drops from 40 percent to zero.".into(), 0.0, 5.2),
            ("It isn't broken.".into(), 5.45, 2.6),
        ]);
        assert_eq!(c.len(), 2);
        assert_eq!(c[0].text, "You walk outside, it's minus ten, and\nyour phone drops from 40 percent to zero.");
        let srt = to_srt(&c);
        assert!(srt.starts_with("1\n00:00:00,000 --> 00:00:05,200\n"), "{srt}");
        assert!(srt.contains("2\n00:00:05,450 --> 00:00:08,050\nIt isn't broken.\n"), "{srt}");
        // a long line becomes several captions that fill its time exactly
        let long = "word ".repeat(40);
        let c = cues(&[(long.trim().into(), 1.0, 10.0)]);
        assert!(c.len() >= 2);
        assert!((c.last().unwrap().end - 11.0).abs() < 1e-9);
        assert!(c.iter().all(|x| x.text.lines().all(|r| r.chars().count() <= CAPTION_ROW)));
    }
}
