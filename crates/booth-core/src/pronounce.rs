//! The pronunciation list ("Say it like this"): words the voice should say differently, like
//! "SQL" as "sequel". Applied to the text sent to the engine (see `Zonos::generate`), so every
//! take, script line and Claude request gets it, while takes and scripts keep what was typed.

use serde::{Deserialize, Serialize};

/// One replacement: where the text has `write`, the voice says `say`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Pronunciation {
    /// The word or phrase as it is written.
    pub write: String,
    /// How to say it, spelled the way it sounds.
    pub say: String,
    /// Only when the case matches too ("US" but not "us").
    pub match_case: bool,
    /// Only for this language (a server code like `es`, `fr_fr`, or a family like `en`); None = all.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
}

/// `text` as the engine should get it, with the saved list applied; None when nothing changed.
pub fn for_engine(text: &str, language: Option<&str>) -> Option<String> {
    let list = crate::settings::load().pronunciations;
    if list.is_empty() {
        return None;
    }
    let out = apply(text, &list, language);
    (out != text).then_some(out)
}

/// Replace every listed word or phrase in one pass, left to right: at each place the longest
/// entry that fits wins, and a replacement is never looked at again.
pub fn apply(text: &str, list: &[Pronunciation], language: Option<&str>) -> String {
    let mut entries: Vec<(&str, &str, bool)> = list
        .iter()
        .filter(|p| language_matches(p.language.as_deref(), language))
        .map(|p| (p.write.trim(), p.say.trim(), p.match_case))
        // A row with nothing to say yet is still being written: leave the word alone.
        .filter(|(w, s, _)| !w.is_empty() && !s.is_empty())
        .collect();
    if entries.is_empty() {
        return text.to_string();
    }
    // Longest first; for the same length, a Match case entry before a loose one.
    entries.sort_by(|a, b| b.0.chars().count().cmp(&a.0.chars().count()).then(b.2.cmp(&a.2)));

    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    'scan: while i < text.len() {
        for &(write, say, match_case) in &entries {
            if let Some(end) = match_at(text, i, write, match_case) {
                out.push_str(say);
                i = end;
                continue 'scan;
            }
        }
        let c = text[i..].chars().next().expect("i is on a char boundary");
        out.push(c);
        i += c.len_utf8();
    }
    out
}

/// Where `write` ends if it is in `text` at byte `i` as a whole word or phrase.
fn match_at(text: &str, i: usize, write: &str, match_case: bool) -> Option<usize> {
    let mut rest = text[i..].char_indices();
    let mut end = i;
    for w in write.chars() {
        let (at, t) = rest.next()?;
        let same = t == w || (!match_case && t.to_lowercase().eq(w.to_lowercase()));
        if !same {
            return None;
        }
        end = i + at + t.len_utf8();
    }
    // A word edge has to stand alone: "SQL" not inside "MySQL" or "SQLite". Next to a letter of
    // a script written without spaces it still counts ("SQLを使う").
    let first = write.chars().next()?;
    let last = write.chars().next_back()?;
    if is_spaced_word_char(first) && text[..i].chars().next_back().is_some_and(is_spaced_word_char) {
        return None;
    }
    if is_spaced_word_char(last) && text[end..].chars().next().is_some_and(is_spaced_word_char) {
        return None;
    }
    Some(end)
}

/// A letter or digit of a script that puts spaces between words. Japanese, Chinese, Korean,
/// Thai and the like don't, so entries in those match anywhere.
fn is_spaced_word_char(c: char) -> bool {
    c.is_alphanumeric() && !is_unspaced_script(c)
}

fn is_unspaced_script(c: char) -> bool {
    matches!(c as u32,
        0x0E00..=0x0EFF      // Thai, Lao
        | 0x1000..=0x109F    // Myanmar
        | 0x1100..=0x11FF    // Hangul Jamo
        | 0x1780..=0x17FF    // Khmer
        | 0x3040..=0x30FF    // Hiragana, Katakana
        | 0x3130..=0x318F    // Hangul compatibility Jamo
        | 0x31F0..=0x31FF    // Katakana extensions
        | 0x3400..=0x4DBF    // CJK extension A
        | 0x4E00..=0x9FFF    // CJK
        | 0xAC00..=0xD7AF    // Hangul syllables
        | 0xF900..=0xFAFF    // CJK compatibility
        | 0xFF66..=0xFF9F    // half-width Katakana
        | 0x20000..=0x3FFFF  // CJK extensions B and on
    )
}

/// An entry's language fits the request's: same code, or one is the family of the other
/// (`en` fits `en_us` and `en_gb`). No language on the entry fits all; no language on the
/// request means US English.
fn language_matches(entry: Option<&str>, request: Option<&str>) -> bool {
    let norm = |s: &str| s.trim().to_lowercase().replace('-', "_");
    let entry = match entry.map(norm) {
        Some(e) if !e.is_empty() => e,
        _ => return true,
    };
    let request = norm(request.unwrap_or("en_us"));
    entry == request
        || request.strip_prefix(&entry).is_some_and(|r| r.starts_with('_'))
        || entry.strip_prefix(&request).is_some_and(|r| r.starts_with('_'))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(write: &str, say: &str) -> Pronunciation {
        Pronunciation { write: write.into(), say: say.into(), ..Default::default() }
    }

    fn run(text: &str, list: &[Pronunciation]) -> String {
        apply(text, list, None)
    }

    #[test]
    fn whole_words_only() {
        let list = [p("SQL", "sequel")];
        assert_eq!(run("SQL and MySQL and SQLite", &list), "sequel and MySQL and SQLite");
    }

    #[test]
    fn ignores_case_unless_asked() {
        let list = [p("gif", "jif")];
        assert_eq!(run("A GIF, a Gif, a gif.", &list), "A jif, a jif, a jif.");
        let strict = [Pronunciation { match_case: true, ..p("US", "you ess") }];
        assert_eq!(run("Made in the US, for us.", &strict), "Made in the you ess, for us.");
    }

    #[test]
    fn punctuation_next_to_words() {
        let list = [p("SQL", "sequel")];
        assert_eq!(
            run("(SQL) SQL, SQL. SQL! \"SQL\" SQL's SQL-based", &list),
            "(sequel) sequel, sequel. sequel! \"sequel\" sequel's sequel-based"
        );
    }

    #[test]
    fn longest_first() {
        let list = [p("New York", "the big apple"), p("New York City", "N Y C"), p("York", "yorrk")];
        assert_eq!(run("New York City, New York, York.", &list), "N Y C, the big apple, yorrk.");
    }

    #[test]
    fn never_replaces_inside_a_replacement() {
        let list = [p("NoFaceTube", "no face tube"), p("face", "FACE"), p("tube", "toob")];
        assert_eq!(run("Welcome to NoFaceTube.", &list), "Welcome to no face tube.");
        // A swap stays a swap.
        let swap = [p("tomato", "tomahto"), p("tomahto", "tomato")];
        assert_eq!(run("tomato tomahto", &swap), "tomahto tomato");
    }

    #[test]
    fn digits_and_symbols() {
        let list = [p("2x", "two times"), p("C++", "C plus plus")];
        assert_eq!(run("Play at 2x, not 12x. I write C++.", &list), "Play at two times, not 12x. I write C plus plus.");
    }

    #[test]
    fn scripts_without_spaces() {
        let list = [p("東京", "とうきょう"), p("SQL", "シークェル")];
        assert_eq!(run("東京都でSQLを使う", &list), "とうきょう都でシークェルを使う");
    }

    #[test]
    fn other_alphabets_keep_word_edges() {
        let list = [p("ООН", "о-о-эн")];
        assert_eq!(run("ООН и ООНовцы", &list), "о-о-эн и ООНовцы");
    }

    #[test]
    fn accents_and_case_folding() {
        let list = [p("café", "kaff ay")];
        assert_eq!(run("CAFÉ. Café!", &list), "kaff ay. kaff ay!");
    }

    #[test]
    fn languages() {
        let list = [
            Pronunciation { language: Some("en".into()), ..p("GIF", "jif") },
            Pronunciation { language: Some("es".into()), ..p("GIF", "guif") },
            p("Booth", "booth"),
        ];
        assert_eq!(apply("GIF", &list, None), "jif");
        assert_eq!(apply("GIF", &list, Some("en_gb")), "jif");
        assert_eq!(apply("GIF", &list, Some("es")), "guif");
        assert_eq!(apply("GIF Booth", &list, Some("fr_fr")), "GIF booth");
        assert!(language_matches(Some("fr"), Some("fr_fr")));
        assert!(!language_matches(Some("fr"), Some("fi")));
    }

    #[test]
    fn blank_entries_and_empty_say() {
        let list = [p("  ", "nothing"), p(" um ", "")];
        assert_eq!(run("So um, yes.", &list), "So um, yes.");
        assert_eq!(run("no list here", &[]), "no list here");
    }

    #[test]
    fn old_settings_still_load() {
        let s: crate::settings::Settings = serde_json::from_str(r#"{"laci": true}"#).unwrap();
        assert!(s.pronunciations.is_empty());
        let s: crate::settings::Settings =
            serde_json::from_str(r#"{"pronunciations": [{"write": "SQL", "say": "sequel"}]}"#).unwrap();
        assert_eq!(s.pronunciations, vec![p("SQL", "sequel")]);
    }
}
