//! Opening a document in the Script tab: the words out of .docx, .pdf, .epub, .html, .rtf, .md
//! and plain text, and subtitles (.srt, .vtt) and screenplays (.fountain) turned into the kind of
//! script `script::parse` reads. Headings come out as `# Heading` lines (the parser leaves them
//! out of the audio and starts a new paragraph after them) and paragraphs keep a blank line
//! between them.
//!
//! Everything here is pure Rust so it builds for Android too, where the file picker hands over
//! the file's bytes rather than a path.

use std::io::{Cursor, Read};
use std::path::Path;

use anyhow::{bail, Context, Result};

/// A file bigger than this isn't a script.
pub const MAX_BYTES: u64 = 100 * 1024 * 1024;

/// What the Script tab opens (the file picker's list).
pub const EXTENSIONS: &[&str] = &[
    "txt", "text", "md", "markdown", "srt", "vtt", "fountain", "docx", "pdf", "epub", "html", "htm", "xhtml", "rtf",
];

/// A PDF with no text layer (a scan or photos of pages).
pub const NO_PDF_TEXT: &str = "This PDF is scanned images, so there's no text to read.";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Text,
    Markdown,
    Subtitles,
    Fountain,
    Html,
    Rtf,
    Docx,
    Epub,
    Pdf,
}

/// The text of a file on disk (a file dropped on the window).
pub fn open_path(path: &Path) -> Result<String> {
    let meta = std::fs::metadata(path).with_context(|| format!("can't open {}", path.display()))?;
    if meta.is_dir() {
        bail!("that's a folder, not a file");
    }
    if meta.len() > MAX_BYTES {
        bail!("that file is too big to be a script (over 100 MB)");
    }
    let bytes = std::fs::read(path).with_context(|| format!("can't read {}", path.display()))?;
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    open_bytes(&name, &bytes)
}

/// The text of a file's bytes; `name` (with its extension) says what kind of file it is.
pub fn open_bytes(name: &str, bytes: &[u8]) -> Result<String> {
    if bytes.len() as u64 > MAX_BYTES {
        bail!("that file is too big to be a script (over 100 MB)");
    }
    let ext = name.rsplit_once('.').map(|(_, e)| e.to_lowercase()).unwrap_or_default();
    let kind = match ext.as_str() {
        "txt" | "text" => Kind::Text,
        "md" | "markdown" => Kind::Markdown,
        "srt" | "vtt" => Kind::Subtitles,
        "fountain" | "spmd" => Kind::Fountain,
        "html" | "htm" | "xhtml" => Kind::Html,
        "rtf" => Kind::Rtf,
        "docx" => Kind::Docx,
        "epub" => Kind::Epub,
        "pdf" => Kind::Pdf,
        "doc" => bail!("Booth can't read old .doc files. Save it as .docx or .txt and open that."),
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "heic" | "mp3" | "wav" | "m4a" | "flac" | "ogg" | "mp4" | "mov" | "zip"
        | "exe" => bail!("Booth can't open .{ext} files in Script. It opens {}.", list_kinds()),
        "pages" | "odt" => bail!("Booth can't read .{ext} files. Save it as .docx, .pdf or .txt and open that."),
        _ => sniff(bytes).with_context(|| {
            format!("Booth can't open {}. It opens {}.", if ext.is_empty() { "that file" } else { "this kind of file" }, list_kinds())
        })?,
    };
    let text = match kind {
        Kind::Text => decode_text(bytes),
        Kind::Markdown => markdown(&decode_text(bytes)),
        Kind::Subtitles => subtitles(&decode_text(bytes)),
        Kind::Fountain => fountain(&decode_text(bytes)),
        Kind::Html => html_to_text(&decode_text(bytes)),
        Kind::Rtf => rtf_to_text(bytes),
        Kind::Docx => docx(bytes)?,
        Kind::Epub => epub(bytes)?,
        Kind::Pdf => pdf(bytes)?,
    };
    let text = tidy(&text);
    if text.trim().is_empty() {
        bail!("there's no text in that file");
    }
    Ok(text)
}

fn list_kinds() -> &'static str {
    ".txt, .docx, .pdf, .epub, .md, .html, .rtf, .srt, .vtt and .fountain"
}

/// What a file without a known extension is, from its first bytes.
fn sniff(bytes: &[u8]) -> Option<Kind> {
    let head = &bytes[..bytes.len().min(512)];
    if head.starts_with(b"%PDF") {
        return Some(Kind::Pdf);
    }
    if head.starts_with(b"PK\x03\x04") {
        let zip = zip::ZipArchive::new(Cursor::new(bytes)).ok()?;
        let has = |n: &str| zip.file_names().any(|f| f == n);
        return if has("word/document.xml") {
            Some(Kind::Docx)
        } else if has("META-INF/container.xml") {
            Some(Kind::Epub)
        } else {
            None
        };
    }
    if head.starts_with(b"{\\rtf") {
        return Some(Kind::Rtf);
    }
    if head.contains(&0) {
        return None; // binary
    }
    let text = decode_text(head);
    let low = text.trim_start().to_lowercase();
    if low.starts_with("<!doctype html") || low.starts_with("<html") {
        return Some(Kind::Html);
    }
    // Text, as long as it isn't binary (control characters other than whitespace).
    let odd = text.chars().filter(|c| c.is_control() && !c.is_whitespace()).count();
    (odd * 50 <= text.chars().count().max(1)).then_some(Kind::Text)
}

/// Text in whatever encoding a text editor saved it in: UTF-8, UTF-16 with a byte-order mark
/// (Windows Notepad's "Unicode"), or Windows-1252 for old files.
pub fn decode_text(bytes: &[u8]) -> String {
    if let Some((enc, bom)) = encoding_rs::Encoding::for_bom(bytes) {
        return enc.decode_without_bom_handling(&bytes[bom..]).0.into_owned();
    }
    match std::str::from_utf8(bytes) {
        Ok(s) => s.to_string(),
        Err(e) if e.error_len().is_none() && e.valid_up_to() + 4 > bytes.len() => {
            // cut off mid-character (the sniffing above reads only the start)
            String::from_utf8_lossy(bytes).into_owned()
        }
        Err(_) => {
            // UTF-16 without a mark has a zero byte in every other place for Latin text.
            let zeros = bytes.iter().skip(1).step_by(2).filter(|b| **b == 0).count();
            if bytes.len() >= 4 && zeros * 3 > bytes.len() / 2 {
                encoding_rs::UTF_16LE.decode_without_bom_handling(bytes).0.into_owned()
            } else {
                encoding_rs::WINDOWS_1252.decode_without_bom_handling(bytes).0.into_owned()
            }
        }
    }
}

/// Line ends, invisible characters and runs of blank lines, the same for every kind of file.
fn tidy(text: &str) -> String {
    let text = text.replace("\r\n", "\n").replace('\r', "\n");
    let mut out = String::with_capacity(text.len());
    let mut blanks = 0;
    for line in text.lines() {
        let line: String = line
            .chars()
            .filter(|c| !matches!(c, '\u{ad}' | '\u{200b}' | '\u{200c}' | '\u{200d}' | '\u{feff}' | '\u{2060}'))
            .map(|c| if matches!(c, '\u{a0}' | '\u{2007}' | '\u{202f}' | '\t') { ' ' } else { c })
            .collect();
        let line = line.trim_end();
        if line.trim().is_empty() || line.trim() == "#" || line.trim() == "-" {
            blanks += 1;
            continue;
        }
        if !out.is_empty() {
            out.push_str(if blanks > 0 { "\n\n" } else { "\n" });
        }
        blanks = 0;
        out.push_str(line);
    }
    out
}

// ---------- markup: a forgiving tokenizer for HTML and the XML inside .docx and .epub ----------

#[derive(Debug)]
enum Tok<'a> {
    /// `<name attrs>`, `</name>` or `<name/>`; the name is lowercase.
    Tag { name: String, attrs: &'a str, close: bool, empty: bool },
    /// Text between tags, entities not decoded yet.
    Text(&'a str),
}

fn tokenize(s: &str) -> Vec<Tok<'_>> {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let (mut i, mut text_from) = (0, 0);
    fn flush<'a>(out: &mut Vec<Tok<'a>>, s: &'a str, from: usize, to: usize) {
        if to > from {
            out.push(Tok::Text(&s[from..to]));
        }
    }
    while i < b.len() {
        if b[i] != b'<' {
            i += 1;
            continue;
        }
        let rest = &s[i..];
        let skip_to = |pat: &str, from: usize| s[from..].find(pat).map_or(s.len(), |p| from + p + pat.len());
        if rest.starts_with("<!--") {
            flush(&mut out, s, text_from, i);
            i = skip_to("-->", i + 4);
            text_from = i;
            continue;
        }
        if rest.starts_with("<![CDATA[") {
            flush(&mut out, s, text_from, i);
            let end = s[i + 9..].find("]]>").map_or(s.len(), |p| i + 9 + p);
            out.push(Tok::Text(&s[i + 9..end]));
            i = (end + 3).min(s.len());
            text_from = i;
            continue;
        }
        if rest.starts_with("<!") || rest.starts_with("<?") {
            flush(&mut out, s, text_from, i);
            i = skip_to(">", i + 2);
            text_from = i;
            continue;
        }
        let close = rest.starts_with("</");
        let name_at = i + 1 + usize::from(close);
        if !b.get(name_at).is_some_and(u8::is_ascii_alphabetic) {
            i += 1; // a lone "<" in text
            continue;
        }
        let mut j = name_at;
        let mut quote = 0u8;
        while j < b.len() {
            let c = b[j];
            if quote != 0 {
                if c == quote {
                    quote = 0;
                }
            } else if c == b'"' || c == b'\'' {
                quote = c;
            } else if c == b'>' {
                break;
            }
            j += 1;
        }
        flush(&mut out, s, text_from, i);
        let inner = &s[name_at..j.min(s.len())];
        let name_end = inner.find(|c: char| c.is_whitespace() || c == '/').unwrap_or(inner.len());
        let name = inner[..name_end].to_ascii_lowercase();
        let empty = inner.ends_with('/');
        i = (j + 1).min(s.len());
        text_from = i;
        // The insides of <script> and <style> aren't markup or words.
        if !close && !empty && (name == "script" || name == "style") {
            let end = s[i..].to_ascii_lowercase().find(&format!("</{name}")).map_or(s.len(), |p| i + p);
            i = end;
            text_from = end;
        }
        out.push(Tok::Tag { name, attrs: &inner[name_end..], close, empty });
    }
    flush(&mut out, s, text_from, s.len());
    out
}

/// One attribute's value from a tag's attribute text (`name` matched without regard to case).
fn attr(attrs: &str, name: &str) -> Option<String> {
    let b = attrs.as_bytes();
    let mut i = 0;
    while i < b.len() {
        while i < b.len() && (b[i].is_ascii_whitespace() || b[i] == b'/') {
            i += 1;
        }
        let start = i;
        while i < b.len() && !b[i].is_ascii_whitespace() && b[i] != b'=' && b[i] != b'/' {
            i += 1;
        }
        let key = &attrs[start..i];
        while i < b.len() && b[i].is_ascii_whitespace() {
            i += 1;
        }
        let mut value = "";
        if i < b.len() && b[i] == b'=' {
            i += 1;
            while i < b.len() && b[i].is_ascii_whitespace() {
                i += 1;
            }
            if i < b.len() && (b[i] == b'"' || b[i] == b'\'') {
                let q = b[i];
                let from = i + 1;
                i = from;
                while i < b.len() && b[i] != q {
                    i += 1;
                }
                value = &attrs[from..i];
                i += 1;
            } else {
                let from = i;
                while i < b.len() && !b[i].is_ascii_whitespace() {
                    i += 1;
                }
                value = &attrs[from..i];
            }
        }
        if key.eq_ignore_ascii_case(name) {
            return Some(entities(value));
        }
        if start == i {
            i += 1;
        }
    }
    None
}

/// `&amp;`, `&#8217;`, `&rsquo;` and the other entities that turn up in documents.
fn entities(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(p) = rest.find('&') {
        out.push_str(&rest[..p]);
        rest = &rest[p..];
        let end = rest[1..].find(|c: char| c == ';' || c == '&' || c.is_whitespace()).map(|e| e + 1);
        let decoded = end.filter(|e| rest.as_bytes()[*e] == b';' && *e <= 12).and_then(|e| {
            let name = &rest[1..e];
            let c = if let Some(num) = name.strip_prefix('#') {
                let n = match num.strip_prefix(['x', 'X']) {
                    Some(hex) => u32::from_str_radix(hex, 16).ok(),
                    None => num.parse().ok(),
                };
                n.and_then(char::from_u32).map(String::from)
            } else {
                named_entity(name).map(String::from)
            };
            c.map(|c| (c, e + 1))
        });
        match decoded {
            Some((c, len)) => {
                out.push_str(&c);
                rest = &rest[len..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

fn named_entity(name: &str) -> Option<&'static str> {
    Some(match name {
        "amp" => "&",
        "lt" => "<",
        "gt" => ">",
        "quot" => "\"",
        "apos" => "'",
        "nbsp" | "ensp" | "emsp" | "thinsp" => " ",
        "mdash" => "—",
        "ndash" => "–",
        "hellip" => "…",
        "lsquo" => "‘",
        "rsquo" => "’",
        "ldquo" => "“",
        "rdquo" => "”",
        "laquo" => "«",
        "raquo" => "»",
        "bull" => "•",
        "middot" => "·",
        "copy" => "©",
        "reg" => "®",
        "trade" => "™",
        "deg" => "°",
        "euro" => "€",
        "pound" => "£",
        "times" => "×",
        "eacute" => "é",
        "egrave" => "è",
        "aacute" => "á",
        "agrave" => "à",
        "iacute" => "í",
        "oacute" => "ó",
        "uacute" => "ú",
        "ntilde" => "ñ",
        "ccedil" => "ç",
        "uuml" => "ü",
        "ouml" => "ö",
        "auml" => "ä",
        "szlig" => "ß",
        "shy" => "",
        _ => return None,
    })
}

/// Words out of a web page or an e-book chapter: headings as `# Heading`, a blank line after
/// each paragraph, and no menus, scripts, styles or footnote marks.
fn html_to_text(s: &str) -> String {
    const SKIP: &[&str] = &[
        "head", "nav", "footer", "aside", "sup", "rt", "rp", "template", "noscript", "svg", "math", "button", "select",
        "iframe", "object", "audio", "video", "canvas", "figure",
    ];
    const BLOCK: &[&str] = &[
        "p", "div", "section", "article", "main", "header", "blockquote", "ul", "ol", "dl", "dt", "dd", "table", "tr",
        "hr", "pre", "body", "address", "figcaption", "caption",
    ];
    let mut out = String::with_capacity(s.len() / 2);
    let mut skip = 0usize;
    let mut heading = false;
    for t in tokenize(s) {
        match t {
            Tok::Tag { name, close, empty, .. } => {
                let name = name.rsplit(':').next().unwrap_or(&name).to_string();
                if SKIP.contains(&name.as_str()) {
                    if !empty {
                        skip = if close { skip.saturating_sub(1) } else { skip + 1 };
                    }
                    continue;
                }
                if skip > 0 {
                    continue;
                }
                match name.as_str() {
                    "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                        heading = !close;
                        out.push_str(if close { "\n\n" } else { "\n\n# " });
                    }
                    "br" if heading => {
                        if !out.ends_with(' ') {
                            out.push(' ');
                        }
                    }
                    "br" => out.push('\n'),
                    "li" if !close => out.push_str("\n- "),
                    "td" | "th" => out.push(' '),
                    n if BLOCK.contains(&n) => out.push_str("\n\n"),
                    _ => {}
                }
            }
            Tok::Text(t) if skip == 0 => {
                let t = entities(t);
                // Whitespace in HTML is one space wherever it falls.
                let mut last_space = out.ends_with([' ', '\n']);
                for c in t.chars() {
                    if c.is_whitespace() && c != '\u{a0}' {
                        if !last_space {
                            out.push(' ');
                        }
                        last_space = true;
                    } else {
                        out.push(c);
                        last_space = false;
                    }
                }
            }
            Tok::Text(_) => {}
        }
    }
    out.lines().map(str::trim).collect::<Vec<_>>().join("\n")
}

// ---------- Word (.docx) ----------

fn read_zip(zip: &mut zip::ZipArchive<Cursor<&[u8]>>, name: &str) -> Result<String> {
    let mut f = zip.by_name(name).with_context(|| format!("{name} is missing"))?;
    let mut bytes = Vec::new();
    f.read_to_end(&mut bytes)?;
    Ok(decode_text(&bytes))
}

/// Style ids that are headings: Word's built-in names are English ("heading 1", "Title") even
/// when the ids are localised ("berschrift1"), and some styles set an outline level.
fn docx_heading_styles(styles: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut id: Option<String> = None;
    let mut is_heading = false;
    for t in tokenize(styles) {
        let Tok::Tag { name, attrs, close, .. } = t else { continue };
        match name.as_str() {
            "w:style" if close => {
                if let Some(id) = id.take().filter(|_| is_heading) {
                    out.push(id);
                }
            }
            "w:style" => {
                id = attr(attrs, "w:styleId");
                is_heading = false;
            }
            "w:name" => {
                let n = attr(attrs, "w:val").unwrap_or_default().to_lowercase();
                is_heading |= n.starts_with("heading") || n == "title" || n == "subtitle";
            }
            "w:outlinelvl" => {
                is_heading |= attr(attrs, "w:val").and_then(|v| v.parse::<u32>().ok()).is_some_and(|v| v < 9);
            }
            _ => {}
        }
    }
    out
}

fn docx(bytes: &[u8]) -> Result<String> {
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).context("this .docx file is damaged")?;
    let xml = read_zip(&mut zip, "word/document.xml").context("this .docx has no document in it")?;
    let headings = read_zip(&mut zip, "word/styles.xml").map(|s| docx_heading_styles(&s)).unwrap_or_default();
    let is_heading_style = |id: &str| {
        let low = id.to_lowercase();
        headings.iter().any(|h| h == id) || low.starts_with("heading") || low == "title"
    };
    let mut out = String::with_capacity(xml.len() / 4);
    let mut para = String::new();
    let mut heading = false;
    let mut in_text = false;
    // Word keeps a copy of text boxes for old readers (mc:Fallback): read only one of them.
    let mut fallback = 0usize;
    let flush = |out: &mut String, para: &mut String, heading: bool| {
        let p = para.trim();
        if !p.is_empty() {
            if heading {
                out.push_str("\n\n# ");
                out.push_str(&p.replace('\n', " "));
            } else {
                out.push_str("\n\n");
                out.push_str(p);
            }
        }
        para.clear();
    };
    for t in tokenize(&xml) {
        match t {
            Tok::Tag { name, attrs, close, empty } => {
                if name == "mc:fallback" {
                    if !empty {
                        fallback = if close { fallback.saturating_sub(1) } else { fallback + 1 };
                    }
                    continue;
                }
                if fallback > 0 {
                    continue;
                }
                match name.as_str() {
                    "w:p" => {
                        flush(&mut out, &mut para, heading);
                        heading = false;
                    }
                    "w:t" => in_text = !close && !empty,
                    "w:tab" | "w:ptab" if !close => para.push(' '),
                    "w:br" | "w:cr" if !close => para.push('\n'),
                    "w:nobreakhyphen" if !close => para.push('-'),
                    "w:pstyle" => heading |= attr(attrs, "w:val").is_some_and(|v| is_heading_style(&v)),
                    "w:outlinelvl" => {
                        heading |= attr(attrs, "w:val").and_then(|v| v.parse::<u32>().ok()).is_some_and(|v| v < 9)
                    }
                    _ => {}
                }
            }
            Tok::Text(t) if in_text && fallback == 0 => para.push_str(&entities(t)),
            Tok::Text(_) => {}
        }
    }
    flush(&mut out, &mut para, heading);
    Ok(out)
}

// ---------- e-books (.epub) ----------

/// `a/b/../c d.xhtml` from a folder and an href (`%20` and `..` resolved, `#anchor` dropped).
fn zip_path(base: &str, href: &str) -> String {
    let href = href.split('#').next().unwrap_or("");
    let mut bytes = Vec::with_capacity(href.len());
    let hb = href.as_bytes();
    let mut i = 0;
    while i < hb.len() {
        if hb[i] == b'%' && i + 2 < hb.len() {
            if let Some(v) = std::str::from_utf8(&hb[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok()) {
                bytes.push(v);
                i += 3;
                continue;
            }
        }
        bytes.push(hb[i]);
        i += 1;
    }
    let href = String::from_utf8_lossy(&bytes).into_owned();
    let mut parts: Vec<&str> = base.split('/').filter(|p| !p.is_empty()).collect();
    for p in href.split('/') {
        match p {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            p => parts.push(p),
        }
    }
    parts.join("/")
}

fn epub(bytes: &[u8]) -> Result<String> {
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).context("this .epub file is damaged")?;
    let mut chapters: Vec<String> = Vec::new();
    let opf_path = read_zip(&mut zip, "META-INF/container.xml").ok().and_then(|c| {
        tokenize(&c).into_iter().find_map(|t| match t {
            Tok::Tag { name, attrs, close: false, .. } if name == "rootfile" => attr(attrs, "full-path"),
            _ => None,
        })
    });
    if let Some(opf_path) = opf_path {
        let opf = read_zip(&mut zip, &opf_path)?;
        let base = opf_path.rsplit_once('/').map(|(d, _)| d).unwrap_or("");
        let mut manifest: Vec<(String, String)> = Vec::new();
        let mut spine: Vec<String> = Vec::new();
        for t in tokenize(&opf) {
            let Tok::Tag { name, attrs, close: false, .. } = t else { continue };
            match name.rsplit(':').next().unwrap_or("") {
                "item" => {
                    if let (Some(id), Some(href)) = (attr(attrs, "id"), attr(attrs, "href")) {
                        manifest.push((id, href));
                    }
                }
                "itemref" if attr(attrs, "linear").as_deref() != Some("no") => {
                    spine.extend(attr(attrs, "idref"));
                }
                _ => {}
            }
        }
        for id in spine {
            if let Some((_, href)) = manifest.iter().find(|(i, _)| *i == id) {
                if let Ok(html) = read_zip(&mut zip, &zip_path(base, href)) {
                    chapters.push(html_to_text(&html));
                }
            }
        }
    }
    if chapters.is_empty() {
        // No usable reading order: every page in the book, in file order.
        let mut names: Vec<String> =
            zip.file_names().filter(|n| n.ends_with(".xhtml") || n.ends_with(".html") || n.ends_with(".htm")).map(String::from).collect();
        names.sort();
        for n in names {
            chapters.push(html_to_text(&read_zip(&mut zip, &n)?));
        }
    }
    Ok(chapters.join("\n\n"))
}

// ---------- PDF ----------

fn pdf(bytes: &[u8]) -> Result<String> {
    // pdf-extract can panic on unusual files; a bad PDF must not take the app down.
    let pages = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| pdf_extract::extract_text_from_mem_by_pages(bytes)));
    let pages = match pages {
        Ok(Ok(p)) => p,
        Ok(Err(e)) => {
            let e = e.to_string();
            if e.to_lowercase().contains("crypt") || e.to_lowercase().contains("password") {
                bail!("This PDF is locked with a password. Open it, save a copy without one, and try that.");
            }
            bail!("Booth couldn't read this PDF ({e})");
        }
        Err(_) => bail!("Booth couldn't read this PDF. Try saving it as text or .docx first."),
    };
    let text = pdf_pages(&pages);
    if text.chars().filter(|c| c.is_alphanumeric()).count() < 20 {
        bail!(NO_PDF_TEXT);
    }
    Ok(text)
}

/// A PDF's pages as flowing text: lines wrapped by the page joined back into paragraphs, words
/// hyphenated at a line end put back together, and bare page numbers left out.
fn pdf_pages(pages: &[String]) -> String {
    let mut out = String::new();
    let mut para = String::new();
    let mut prev = String::new();
    let flush = |out: &mut String, para: &mut String| {
        if !para.trim().is_empty() {
            out.push_str(para.trim());
            out.push_str("\n\n");
        }
        para.clear();
    };
    let page_number = |l: &str| {
        let low = l.to_lowercase();
        let low = low.trim_start_matches("page").trim();
        !low.is_empty() && low.split(" of ").all(|n| !n.is_empty() && n.trim().chars().all(|c| c.is_ascii_digit()))
    };
    for page in pages {
        for line in page.lines() {
            let line = line.trim();
            if line.is_empty() {
                flush(&mut out, &mut para);
                continue;
            }
            if page_number(line) {
                continue;
            }
            let prev_line = std::mem::replace(&mut prev, line.to_string());
            if para.is_empty() {
                para.push_str(line);
                continue;
            }
            // A short line that doesn't end in punctuation is a heading or a list item: keep it
            // apart. Anything else is a line the page wrapped.
            let prev = prev_line.as_str();
            let open = !prev.trim_end().ends_with(['.', ',', ';', ':', '!', '?', '"', '\'', '”', '’', ')', '…', '-']);
            if prev.chars().count() < 50 && open {
                flush(&mut out, &mut para);
                para.push_str(line);
            } else if para.ends_with('-')
                && para[..para.len() - 1].chars().last().is_some_and(char::is_alphabetic)
                && line.chars().next().is_some_and(char::is_lowercase)
            {
                para.pop();
                para.push_str(line);
            } else {
                para.push(' ');
                para.push_str(line);
            }
        }
    }
    flush(&mut out, &mut para);
    out
}

// ---------- RTF ----------

/// The words in an RTF file (WordPad, TextEdit, older word processors), with `\par` as a
/// paragraph break and the font, colour, style and picture tables left out.
fn rtf_to_text(bytes: &[u8]) -> String {
    const DESTINATIONS: &[&str] = &[
        "fonttbl", "colortbl", "stylesheet", "info", "pict", "header", "headerl", "headerr", "headerf", "footer",
        "footerl", "footerr", "footerf", "footnote", "fldinst", "object", "listtable", "listoverridetable", "rsidtbl",
        "generator", "xmlnstbl", "themedata", "colorschememapping", "latentstyles", "datastore", "filetbl", "revtbl",
        "pgdsctbl", "mmathpr", "annotation", "atnid", "atnauthor", "bkmkstart", "bkmkend", "shp", "shpinst",
        "nonshppict", "comment", "userprops", "operator", "author", "title",
    ];
    #[derive(Clone, Copy)]
    struct Group {
        skip: bool,
        uc: usize,
    }
    let mut out = String::new();
    let mut pending: Vec<u8> = Vec::new();
    let mut enc = encoding_rs::WINDOWS_1252;
    let mut stack = vec![Group { skip: false, uc: 1 }];
    let mut fallback_left = 0usize; // characters after \uN that stand in for it in old readers
    let mut i = 0;
    let push_str = |out: &mut String, pending: &mut Vec<u8>, enc: &'static encoding_rs::Encoding, s: &str| {
        if !pending.is_empty() {
            out.push_str(&enc.decode_without_bom_handling(pending).0);
            pending.clear();
        }
        out.push_str(s);
    };
    while i < bytes.len() {
        let top = *stack.last().unwrap_or(&Group { skip: false, uc: 1 });
        match bytes[i] {
            b'{' => {
                stack.push(top);
                i += 1;
            }
            b'}' => {
                if stack.len() > 1 {
                    stack.pop();
                }
                i += 1;
            }
            b'\\' => {
                let next = bytes.get(i + 1).copied().unwrap_or(b' ');
                match next {
                    b'\\' | b'{' | b'}' => {
                        if fallback_left > 0 {
                            fallback_left -= 1;
                        } else if !top.skip {
                            pending.push(next);
                        }
                        i += 2;
                    }
                    b'\'' => {
                        let hex = bytes.get(i + 2..i + 4).and_then(|h| std::str::from_utf8(h).ok());
                        if let Some(b) = hex.and_then(|h| u8::from_str_radix(h, 16).ok()) {
                            if fallback_left > 0 {
                                fallback_left -= 1;
                            } else if !top.skip {
                                pending.push(b);
                            }
                        }
                        i += 4;
                    }
                    b'*' => {
                        if let Some(g) = stack.last_mut() {
                            g.skip = true;
                        }
                        i += 2;
                    }
                    b'~' if !top.skip => {
                        pending.push(b' ');
                        i += 2;
                    }
                    b'_' if !top.skip => {
                        pending.push(b'-');
                        i += 2;
                    }
                    b'\n' | b'\r' if !top.skip => {
                        push_str(&mut out, &mut pending, enc, "\n\n");
                        i += 2;
                    }
                    c if c.is_ascii_alphabetic() => {
                        let start = i + 1;
                        let mut j = start;
                        while j < bytes.len() && bytes[j].is_ascii_alphabetic() {
                            j += 1;
                        }
                        let word = std::str::from_utf8(&bytes[start..j]).unwrap_or("");
                        let num_start = j;
                        if j < bytes.len() && bytes[j] == b'-' {
                            j += 1;
                        }
                        while j < bytes.len() && bytes[j].is_ascii_digit() {
                            j += 1;
                        }
                        let param: Option<i32> = std::str::from_utf8(&bytes[num_start..j]).ok().and_then(|n| n.parse().ok());
                        if j < bytes.len() && bytes[j] == b' ' {
                            j += 1;
                        }
                        i = j;
                        if DESTINATIONS.contains(&word) {
                            if let Some(g) = stack.last_mut() {
                                g.skip = true;
                            }
                            continue;
                        }
                        match word {
                            "uc" => {
                                if let Some(g) = stack.last_mut() {
                                    g.uc = param.unwrap_or(1).max(0) as usize;
                                }
                            }
                            "ansicpg" => {
                                let label = match param {
                                    Some(65001) => "utf-8".to_string(),
                                    Some(10000) => "macintosh".to_string(),
                                    Some(n) => format!("windows-{n}"),
                                    None => "windows-1252".to_string(),
                                };
                                enc = encoding_rs::Encoding::for_label(label.as_bytes()).unwrap_or(encoding_rs::WINDOWS_1252);
                            }
                            _ if top.skip => {}
                            "u" => {
                                let n = param.unwrap_or(0);
                                let n = if n < 0 { n + 65536 } else { n } as u32;
                                if let Some(c) = char::from_u32(n) {
                                    push_str(&mut out, &mut pending, enc, c.encode_utf8(&mut [0; 4]));
                                }
                                fallback_left = top.uc;
                            }
                            "par" | "sect" | "page" => push_str(&mut out, &mut pending, enc, "\n\n"),
                            "line" | "row" => push_str(&mut out, &mut pending, enc, "\n"),
                            "tab" | "cell" | "emspace" | "enspace" | "qmspace" => push_str(&mut out, &mut pending, enc, " "),
                            "emdash" => push_str(&mut out, &mut pending, enc, "—"),
                            "endash" => push_str(&mut out, &mut pending, enc, "–"),
                            "lquote" => push_str(&mut out, &mut pending, enc, "‘"),
                            "rquote" => push_str(&mut out, &mut pending, enc, "’"),
                            "ldblquote" => push_str(&mut out, &mut pending, enc, "“"),
                            "rdblquote" => push_str(&mut out, &mut pending, enc, "”"),
                            "bullet" => push_str(&mut out, &mut pending, enc, "•"),
                            _ => {}
                        }
                    }
                    _ => i += 2,
                }
            }
            b'\r' | b'\n' => i += 1,
            c => {
                if fallback_left > 0 {
                    fallback_left -= 1;
                } else if !top.skip {
                    pending.push(c);
                }
                i += 1;
            }
        }
    }
    push_str(&mut out, &mut pending, enc, "");
    out
}

// ---------- Markdown ----------

/// Markdown as the parser wants it: `[text](link)` as its text, images and code blocks left out,
/// `Title\n=====` headings as `# Title`, and front matter's title as the heading.
fn markdown(text: &str) -> String {
    let text = text.replace("\r\n", "\n");
    let mut lines: Vec<String> = Vec::new();
    let mut src = text.lines().peekable();
    // YAML front matter
    if text.starts_with("---\n") {
        src.next();
        for l in src.by_ref() {
            if l.trim() == "---" || l.trim() == "..." {
                break;
            }
            if let Some(t) = l.strip_prefix("title:") {
                lines.push(format!("# {}", t.trim().trim_matches(['"', '\''])));
            }
        }
    }
    let mut fence = false;
    for l in src {
        let t = l.trim();
        if t.starts_with("```") || t.starts_with("~~~") {
            fence = !fence;
            continue;
        }
        if fence {
            continue;
        }
        // reference definitions: [1]: https://...
        if t.starts_with('[') && t.find("]: ").is_some_and(|p| p > 1 && !t[1..p].contains(['[', ']']) && !t[p + 3..].trim().contains(' ')) {
            continue;
        }
        let setext = !t.is_empty() && t.len() >= 3 && (t.chars().all(|c| c == '=') || t.chars().all(|c| c == '-'));
        if setext {
            if let Some(prev) = lines.last_mut().filter(|p| !p.trim().is_empty() && !p.starts_with('#')) {
                *prev = format!("# {}", prev.trim());
                lines.push(String::new());
                continue;
            }
        }
        lines.push(md_inline(l));
    }
    lines.join("\n")
}

fn md_inline(l: &str) -> String {
    let chars: Vec<char> = l.chars().collect();
    let mut out = String::with_capacity(l.len());
    let mut i = 0;
    while i < chars.len() {
        let image = chars[i] == '!' && chars.get(i + 1) == Some(&'[');
        if chars[i] == '[' || image {
            let open = if image { i + 1 } else { i };
            let close = chars[open..].iter().position(|&c| c == ']').map(|p| open + p);
            if let Some(close) = close.filter(|c| chars.get(c + 1) == Some(&'(')) {
                if let Some(end) = chars[close..].iter().position(|&c| c == ')').map(|p| close + p) {
                    if !image {
                        out.extend(&chars[open + 1..close]);
                    }
                    i = end + 1;
                    continue;
                }
            }
        }
        if chars[i] != '`' {
            out.push(chars[i]);
        }
        i += 1;
    }
    // <!-- comments --> on one line
    while let (Some(a), Some(b)) = (out.find("<!--"), out.find("-->")) {
        if b < a {
            break;
        }
        out.replace_range(a..b + 3, "");
    }
    out
}

// ---------- subtitles (.srt, .vtt) ----------

/// `00:00:01,000 --> 00:00:03,500` (SubRip) or `00:01.000 --> 00:03.500 align:start` (WebVTT),
/// as start and end in seconds.
fn cue_timing(line: &str) -> Option<(f64, f64)> {
    let (a, b) = line.split_once("-->")?;
    let b = b.split_whitespace().next()?;
    Some((stamp(a.trim())?, stamp(b)?))
}

fn stamp(s: &str) -> Option<f64> {
    let s = s.replace(',', ".");
    let parts: Vec<&str> = s.split(':').collect();
    if parts.len() < 2 || parts.len() > 3 {
        return None;
    }
    let mut t = 0.0;
    for p in &parts {
        if p.is_empty() || !p.chars().all(|c| c.is_ascii_digit() || c == '.') {
            return None;
        }
        t = t * 60.0 + p.parse::<f64>().ok()?;
    }
    Some(t)
}

/// Pasted or opened subtitles (SubRip or WebVTT), told apart from a script by their first cue.
pub fn looks_like_subtitles(text: &str) -> bool {
    let mut lines = text.lines().map(|l| l.trim().trim_start_matches('\u{feff}')).filter(|l| !l.is_empty());
    match (lines.next(), lines.next()) {
        (Some(first), _) if first.starts_with("WEBVTT") => true,
        (Some(first), Some(second)) => {
            cue_timing(first).is_some() || first.chars().all(|c| c.is_ascii_digit()) && cue_timing(second).is_some()
        }
        _ => false,
    }
}

/// The words of subtitles as a script: cue numbers, timings and styling left out, cues that run
/// on joined into sentences, a new paragraph after a pause of two seconds or more, and WebVTT
/// voices (`<v Roger>`) as speaker labels.
pub fn subtitles(text: &str) -> String {
    let text = text.replace("\r\n", "\n").replace('\r', "\n");
    let lines: Vec<&str> = text.lines().map(|l| l.trim().trim_start_matches('\u{feff}')).collect();
    struct Cue {
        start: f64,
        end: f64,
        lines: Vec<String>,
    }
    let mut cues: Vec<Cue> = Vec::new();
    let mut i = 0;
    let mut in_cue = false;
    while i < lines.len() {
        let l = lines[i];
        if l.is_empty() {
            in_cue = false;
            i += 1;
            continue;
        }
        if let Some((start, end)) = cue_timing(l) {
            cues.push(Cue { start, end, lines: Vec::new() });
            in_cue = true;
        } else if in_cue && l.chars().all(|c| c.is_ascii_digit()) && lines.get(i + 1).is_some_and(|n| cue_timing(n).is_some()) {
            // the next cue's number, with no blank line before it
        } else if in_cue {
            if let Some(c) = cues.last_mut() {
                c.lines.push(l.to_string());
            }
        } else if l.starts_with("WEBVTT") || l.starts_with("NOTE") || l.starts_with("STYLE") || l.starts_with("REGION") {
            // a header or a block of notes: skip to the next blank line
            while i < lines.len() && !lines[i].is_empty() {
                i += 1;
            }
            continue;
        }
        // anything else outside a cue is a cue number or id
        i += 1;
    }

    let mut out = String::new();
    let mut prev_end: Option<f64> = None;
    let mut prev_speaker: Option<String> = None;
    for c in cues {
        let mut first = true;
        for raw in &c.lines {
            let (speaker, words) = cue_text(raw);
            if words.is_empty() {
                continue;
            }
            let dash = words.starts_with('-');
            let words = words.trim_start_matches(['-', '–']).trim().to_string();
            let speaker_changed = speaker.is_some() && speaker != prev_speaker;
            let sep = if out.is_empty() {
                ""
            } else if first && prev_end.is_some_and(|e| c.start - e >= 2.0) {
                "\n\n"
            } else if dash || speaker_changed {
                "\n"
            } else {
                " "
            };
            out.push_str(sep);
            if let Some(s) = speaker.as_ref().filter(|_| speaker_changed || sep.contains('\n')) {
                out.push_str(s);
                out.push_str(": ");
            }
            out.push_str(&words);
            if speaker.is_some() {
                prev_speaker = speaker;
            }
            first = false;
        }
        prev_end = Some(c.end);
    }
    out
}

/// One subtitle line without its styling: `<i>`, `<b>`, `<c.yellow>`, `{\an8}`, timestamps inside
/// karaoke cues. A WebVTT voice tag comes back as the speaker.
fn cue_text(line: &str) -> (Option<String>, String) {
    let mut speaker = None;
    let mut out = String::new();
    let mut rest = line;
    while let Some(p) = rest.find(['<', '{']) {
        out.push_str(&rest[..p]);
        let close = if rest.as_bytes()[p] == b'<' { '>' } else { '}' };
        match rest[p..].find(close) {
            Some(e) => {
                let tag = &rest[p + 1..p + e];
                if let Some(name) = tag.strip_prefix("v ").or_else(|| tag.strip_prefix("v.").and_then(|t| t.split_once(' ').map(|x| x.1))) {
                    speaker = Some(name.trim().to_string()).filter(|n| !n.is_empty());
                }
                rest = &rest[p + e + 1..];
            }
            None => {
                out.push_str(&rest[p..]);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    (speaker, entities(&out).split_whitespace().collect::<Vec<_>>().join(" "))
}

// ---------- screenplays (.fountain) ----------

/// A Fountain screenplay as a script: a character's name sets the speaker for the dialogue under
/// it (`NAME: dialogue`), scene headings, transitions and (parentheticals) become notes, action
/// stays as narration, and the title page's title becomes the heading.
pub fn fountain(text: &str) -> String {
    let text = strip_between(&strip_between(&text.replace("\r\n", "\n"), "/*", "*/"), "[[", "]]");
    let lines: Vec<&str> = text.lines().collect();
    let mut out: Vec<String> = Vec::new();
    let mut i = 0;

    // Title page: "Key: value" lines before the first blank line.
    const KEYS: &[&str] =
        &["title", "credit", "author", "authors", "source", "draft date", "date", "contact", "copyright", "notes", "revision"];
    let is_key = |l: &str| l.split_once(':').is_some_and(|(k, _)| KEYS.contains(&k.trim().to_lowercase().as_str()));
    if lines.first().is_some_and(|l| is_key(l)) {
        let mut title = String::new();
        let mut in_title = false;
        while i < lines.len() && !lines[i].trim().is_empty() {
            let l = lines[i];
            if is_key(l) && !l.starts_with([' ', '\t']) {
                let (k, v) = l.split_once(':').unwrap_or((l, ""));
                in_title = k.trim().eq_ignore_ascii_case("title");
                if in_title {
                    title.push_str(v.trim());
                }
            } else if in_title {
                if !title.is_empty() {
                    title.push(' ');
                }
                title.push_str(l.trim());
            }
            i += 1;
        }
        let title = title.replace(['*', '_'], "");
        if !title.trim().is_empty() {
            out.push(format!("# {}", title.trim()));
            out.push(String::new());
        }
    }

    let blank = |n: usize| lines.get(n).is_none_or(|l| l.trim().is_empty());
    let mut speaker: Option<String> = None;
    let mut speech = String::new();
    let flush = |out: &mut Vec<String>, speaker: &Option<String>, speech: &mut String| {
        if let Some(s) = speaker {
            if !speech.trim().is_empty() {
                out.push(format!("{s}: {}", speech.trim()));
            }
        }
        speech.clear();
    };
    while i < lines.len() {
        let raw = lines[i];
        let l = raw.trim();
        if speaker.is_some() {
            // Dialogue runs until a blank line ("  " with two spaces is a deliberate blank inside it).
            if l.is_empty() && raw != "  " {
                flush(&mut out, &speaker, &mut speech);
                speaker = None;
                out.push(String::new());
            } else if l.starts_with('(') && l.ends_with(')') {
                flush(&mut out, &speaker, &mut speech);
                out.push(l.to_string());
            } else if !l.is_empty() {
                if !speech.is_empty() {
                    speech.push(' ');
                }
                speech.push_str(&fountain_inline(l.trim_start_matches('~')));
            }
            i += 1;
            continue;
        }
        if l.is_empty() {
            out.push(String::new());
            i += 1;
            continue;
        }
        let after_blank = i == 0 || blank(i - 1);
        if l.starts_with("===") {
            out.push(String::new()); // page break
        } else if l.starts_with('=') {
            out.push(format!("[{}]", l.trim_start_matches('=').trim())); // synopsis
        } else if l.starts_with('#') {
            out.push(l.to_string()); // section: a heading
        } else if l.starts_with('>') && l.ends_with('<') {
            out.push(format!("[{}]", l.trim_matches(['>', '<', ' '])));
        } else if after_blank && (is_scene_heading(l) || is_transition(l)) {
            let l = if l.starts_with('.') && !l.starts_with("..") { &l[1..] } else { l.trim_start_matches('>') };
            out.push(format!("[{}]", l.trim()));
        } else if let Some(name) = (after_blank && !blank(i + 1)).then(|| character(l)).flatten() {
            speaker = Some(name);
        } else {
            out.push(fountain_inline(l.strip_prefix('!').unwrap_or(l)));
        }
        i += 1;
    }
    flush(&mut out, &speaker, &mut speech);
    out.join("\n")
}

fn strip_between(s: &str, open: &str, close: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(a) = rest.find(open) {
        out.push_str(&rest[..a]);
        match rest[a + open.len()..].find(close) {
            Some(b) => rest = &rest[a + open.len() + b + close.len()..],
            None => {
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out
}

/// Fountain's emphasis marks (`*italic*`, `**bold**`, `_underline_`) aren't spoken.
fn fountain_inline(l: &str) -> String {
    l.replace(['*', '_'], "").replace("\\", "")
}

fn is_scene_heading(l: &str) -> bool {
    if l.starts_with('.') && !l.starts_with("..") && l.len() > 1 {
        return true;
    }
    let up = l.to_uppercase();
    ["INT", "EXT", "EST", "INT./EXT", "INT/EXT", "I/E"].iter().any(|p| {
        up.strip_prefix(p).is_some_and(|rest| rest.starts_with('.') || rest.starts_with(' '))
    })
}

fn is_transition(l: &str) -> bool {
    (l.starts_with('>') && !l.ends_with('<'))
        || (l.ends_with("TO:") && !l.chars().any(char::is_lowercase))
}

/// `MAYA`, `DR. KOFI (V.O.)`, `@McCLANE` or `BRICK ^` (dual dialogue) as a speaker name, or
/// `None` when the line isn't a character cue.
fn character(l: &str) -> Option<String> {
    let forced = l.starts_with('@');
    let l = l.trim_start_matches('@').trim_end_matches('^').trim();
    let name = match l.find('(') {
        Some(p) => l[..p].trim(),
        None => l,
    };
    if name.is_empty() || !name.chars().any(char::is_alphabetic) {
        return None;
    }
    if !forced && name.chars().any(char::is_lowercase) {
        return None;
    }
    // The script parser's speaker labels have no dots or commas: "DR. KOFI" -> "DR KOFI".
    let clean: String = name.chars().filter(|c| c.is_alphanumeric() || matches!(c, ' ' | '\'' | '-')).collect();
    // Uppercase like most cues, so the parser takes `MCCLANE:` for a speaker at once.
    let clean = clean.split_whitespace().collect::<Vec<_>>().join(" ").to_uppercase();
    (!clean.is_empty() && clean.split_whitespace().count() <= 4).then_some(clean)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::script::parse;

    fn texts(src: &str) -> Vec<(Option<String>, String)> {
        parse(src, &[]).lines.into_iter().map(|l| (l.speaker, l.text)).collect()
    }

    #[test]
    fn srt_and_vtt() {
        let srt = "1\n00:00:01,000 --> 00:00:03,000\nYou walk outside,\nit's minus ten,\n\n2\n00:00:03,100 --> 00:00:05,000\nand your phone <i>dies</i>.\n\n\
3\n00:00:09,000 --> 00:00:11,000\n{\\an8}It isn't broken.\n";
        assert!(looks_like_subtitles(srt));
        let s = subtitles(srt);
        assert_eq!(s, "You walk outside, it's minus ten, and your phone dies.\n\nIt isn't broken.");
        let p = parse(srt, &[]);
        assert_eq!(p.lines.len(), 2);
        assert!(p.lines[1].para);
        assert!(p.skipped.is_empty(), "{:?}", p.skipped);

        let vtt = "WEBVTT - cold phones\nKind: captions\n\nNOTE written by hand\n\nintro\n00:01.000 --> 00:02.500 align:start\n<v Maya>Is it broken?\n\n\
00:02.600 --> 00:04.000\n<v Kofi>No, just <c.yellow>cold</c>.\n\n00:04.100 --> 00:05.000\n<v Kofi>Warm it up.\n\n\
00:05.100 --> 00:06.000\n<v Maya>Oh.\n";
        assert!(looks_like_subtitles(vtt));
        assert_eq!(subtitles(vtt), "Maya: Is it broken?\nKofi: No, just cold. Warm it up.\nMaya: Oh.");
        assert_eq!(
            texts(vtt),
            [
                (Some("Maya".into()), "Is it broken?".into()),
                (Some("Kofi".into()), "No, just cold. Warm it up.".into()),
                (Some("Maya".into()), "Oh.".into())
            ]
        );
        // no blank line between cues
        assert_eq!(subtitles("1\n00:00:01,000 --> 00:00:02,000\nOne\n2\n00:00:02,100 --> 00:00:03,000\ntwo."), "One two.");
        // dialogue dashes start new lines
        assert_eq!(subtitles("1\n00:00:01,000 --> 00:00:02,000\n- Hi.\n- Hello.\n"), "Hi.\nHello.");
        assert!(!looks_like_subtitles("NARRATOR: 10:30 is late.\nIt is."));
    }

    #[test]
    fn fountain_screenplay() {
        let src = "Title: **Cold Phones**\nAuthor: Someone\n\nINT. KITCHEN - NIGHT\n\nMaya stares at her phone. /* cut this? */\n\n\
MAYA\n(quietly)\nIt was at forty percent.\nNow it's dead.\n\nDR. KOFI (V.O.)\nIt isn't broken. [[check the science]]\n\nCUT TO:\n\n\
EXT. STREET - DAY\n\n@McCLANE\nYippee.\n\n> THE END <\n";
        let s = tidy(&fountain(src));
        assert_eq!(
            s,
            "# Cold Phones\n\n[INT. KITCHEN - NIGHT]\n\nMaya stares at her phone.\n\n(quietly)\nMAYA: It was at forty percent. Now it's dead.\n\n\
DR KOFI: It isn't broken.\n\n[CUT TO:]\n\n[EXT. STREET - DAY]\n\nMCCLANE: Yippee.\n\n[THE END]"
        );
        let p = parse(&s, &[]);
        assert_eq!(
            p.lines.iter().map(|l| (l.speaker.as_deref(), l.text.as_str())).collect::<Vec<_>>(),
            [
                (None, "Maya stares at her phone."),
                (Some("MAYA"), "It was at forty percent. Now it's dead."),
                (Some("DR KOFI"), "It isn't broken."),
                (Some("MCCLANE"), "Yippee."),
            ]
        );
        assert_eq!(
            p.skipped,
            ["# Cold Phones", "[INT. KITCHEN - NIGHT]", "(quietly)", "[CUT TO:]", "[EXT. STREET - DAY]", "[THE END]"]
        );
    }

    #[test]
    fn html_and_markdown() {
        let html = "<!DOCTYPE html><html><head><title>x</title><style>p{color:red}</style></head><body>\
<nav><a href=/>Home</a></nav><h1>Cold <br>phones</h1><p>You walk outside,\n   it&rsquo;s minus&nbsp;ten.<sup>1</sup></p>\
<p>It isn&#39;t <b>broken</b> &amp; it&#x27;s fine.</p><script>var a = '<p>no</p>';</script><ul><li>One</li><li>Two</li></ul></body></html>";
        assert_eq!(
            tidy(&html_to_text(html)),
            "# Cold phones\n\nYou walk outside, it’s minus ten.\n\nIt isn't broken & it's fine.\n\n- One\n- Two"
        );
        let md = "---\ntitle: \"Cold\"\n---\nIntro\n=====\n\nSee [the study](https://x.org/a) ![chart](c.png) and `this`.\n\n```\ncode\n```\n[1]: https://x.org\n";
        assert_eq!(tidy(&markdown(md)), "# Cold\n# Intro\n\nSee the study  and this.");
    }

    #[test]
    fn rtf() {
        let rtf = br"{\rtf1\ansi\ansicpg1252\deff0{\fonttbl{\f0 Calibri;}}{\colortbl;\red0\green0\blue0;}{\*\generator Riched20;}\pard\f0\fs22 Caf\'e9 \b open\b0 .\par
It\rquote s \u8220?quoted\u8221?.\par}";
        assert_eq!(tidy(&rtf_to_text(rtf)), "Café open.\n\nIt’s “quoted”.");
    }

    fn zip_of(files: &[(&str, &str)]) -> Vec<u8> {
        let mut buf = Cursor::new(Vec::new());
        {
            let mut w = zip::ZipWriter::new(&mut buf);
            for (name, body) in files {
                w.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
                std::io::Write::write_all(&mut w, body.as_bytes()).unwrap();
            }
            w.finish().unwrap();
        }
        buf.into_inner()
    }

    #[test]
    fn docx_and_epub() {
        let doc = r#"<?xml version="1.0"?><w:document xmlns:w="x"><w:body>
<w:p><w:pPr><w:pStyle w:val="berschrift1"/></w:pPr><w:r><w:t>Cold phones</w:t></w:r></w:p>
<w:p><w:r><w:t xml:space="preserve">NARRATOR: You walk </w:t></w:r><w:r><w:rPr><w:b/></w:rPr><w:t>outside</w:t></w:r><w:del><w:r><w:delText>gone</w:delText></w:r></w:del><w:r><w:t>.</w:t><w:tab/><w:t>It &amp; you.</w:t></w:r></w:p>
<w:p/><w:p><w:r><mc:AlternateContent><mc:Choice><w:t>Box</w:t></mc:Choice><mc:Fallback><w:t>Box</w:t></mc:Fallback></mc:AlternateContent></w:r></w:p>
</w:body></w:document>"#;
        let styles = r#"<w:styles><w:style w:type="paragraph" w:styleId="berschrift1"><w:name w:val="heading 1"/></w:style><w:style w:styleId="Normal"><w:name w:val="Normal"/></w:style></w:styles>"#;
        let bytes = zip_of(&[("word/document.xml", doc), ("word/styles.xml", styles)]);
        assert_eq!(open_bytes("a.docx", &bytes).unwrap(), "# Cold phones\n\nNARRATOR: You walk outside. It & you.\n\nBox");
        // and without the extension, from its contents
        assert_eq!(open_bytes("download", &bytes).unwrap(), "# Cold phones\n\nNARRATOR: You walk outside. It & you.\n\nBox");

        let container = r#"<container><rootfiles><rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/></rootfiles></container>"#;
        let opf = r#"<package><manifest><item id="c2" href="text/ch%202.xhtml" media-type="application/xhtml+xml"/><item id="c1" href="text/ch1.xhtml"/><item id="nav" href="nav.xhtml"/></manifest>
<spine><itemref idref="nav" linear="no"/><itemref idref="c1"/><itemref idref="c2"/></spine></package>"#;
        let bytes = zip_of(&[
            ("mimetype", "application/epub+zip"),
            ("META-INF/container.xml", container),
            ("OEBPS/content.opf", opf),
            ("OEBPS/nav.xhtml", "<html><body><p>Contents</p></body></html>"),
            ("OEBPS/text/ch1.xhtml", "<html><body><h2>One</h2><p>First.</p></body></html>"),
            ("OEBPS/text/ch 2.xhtml", "<html><body><h2>Two</h2><p>Second.</p></body></html>"),
        ]);
        assert_eq!(open_bytes("book.epub", &bytes).unwrap(), "# One\n\nFirst.\n\n# Two\n\nSecond.");
    }

    #[test]
    fn pdf_text_and_scans() {
        let pages = [
            "A heading\nThis line was wrapped by the page because it is long enough to wrap and\nkeeps going here. A word split by a hyph-\nenation mark.\n\n2".to_string(),
            "Next page para.".to_string(),
        ];
        assert_eq!(
            pdf_pages(&pages),
            "A heading\n\nThis line was wrapped by the page because it is long enough to wrap and keeps going here. A word split by a hyphenation mark.\n\nNext page para.\n\n"
        );
        // A real PDF made here: its text comes out; one with no text layer says so.
        let text = open_bytes("a.pdf", &make_pdf(&["You walk outside, it is minus ten.", "Your phone dies."])).unwrap();
        assert_eq!(text, "You walk outside, it is minus ten.\n\nYour phone dies.");
        assert_eq!(open_bytes("scan.pdf", &make_pdf(&[])).unwrap_err().to_string(), NO_PDF_TEXT);
        // not a PDF at all: a clear message, never a panic
        assert!(open_bytes("x.pdf", b"%PDF-1.4 garbage").is_err());
    }

    /// A one-page PDF with each line in Helvetica, well apart (paragraphs).
    fn make_pdf(lines: &[&str]) -> Vec<u8> {
        use pdf_extract::content::{Content, Operation};
        use pdf_extract::{dictionary, Document, Object, Stream};
        let mut doc = Document::with_version("1.5");
        let pages_id = doc.new_object_id();
        let font_id = doc.add_object(dictionary! { "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica" });
        let resources_id = doc.add_object(dictionary! { "Font" => dictionary! { "F1" => font_id } });
        let mut ops = Vec::new();
        for (i, l) in lines.iter().enumerate() {
            ops.push(Operation::new("BT", vec![]));
            ops.push(Operation::new("Tf", vec!["F1".into(), 12.into()]));
            ops.push(Operation::new("Td", vec![72.into(), (700 - 40 * i as i64).into()]));
            ops.push(Operation::new("Tj", vec![Object::string_literal(*l)]));
            ops.push(Operation::new("ET", vec![]));
        }
        let content_id = doc.add_object(Stream::new(dictionary! {}, Content { operations: ops }.encode().unwrap()));
        let page_id = doc.add_object(dictionary! { "Type" => "Page", "Parent" => pages_id, "Contents" => content_id });
        doc.objects.insert(
            pages_id,
            Object::Dictionary(dictionary! {
                "Type" => "Pages", "Kids" => vec![page_id.into()], "Count" => 1,
                "Resources" => resources_id, "MediaBox" => vec![0.into(), 0.into(), 595.into(), 842.into()],
            }),
        );
        let catalog_id = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
        doc.trailer.set("Root", catalog_id);
        let mut out = Vec::new();
        doc.save_to(&mut out).unwrap();
        out
    }

    #[test]
    fn text_encodings_and_unknown_files() {
        assert_eq!(open_bytes("a.txt", "\u{feff}Hi\r\n\r\n\r\nthere".as_bytes()).unwrap(), "Hi\n\nthere");
        let utf16: Vec<u8> = [0xFF, 0xFE].into_iter().chain("Hé".encode_utf16().flat_map(|u| u.to_le_bytes())).collect();
        assert_eq!(open_bytes("a.txt", &utf16).unwrap(), "Hé");
        assert_eq!(open_bytes("a.txt", b"caf\xe9 \x93ok\x94").unwrap(), "café “ok”");
        assert!(open_bytes("a.png", &[0x89, b'P', b'N', b'G', 0, 0, 1, 2, 3, 0, 0, 0]).is_err());
        assert!(open_bytes("a.doc", b"x").unwrap_err().to_string().contains(".docx"));
    }

    /// About 20,000 words of book-like text: headings, paragraphs of 3 to 7 sentences.
    fn long_text() -> String {
        const W: &[&str] = &[
            "the", "phone", "cold", "battery", "walked", "outside", "and", "it", "was", "minus",
            "ten", "charge", "comes", "back", "when", "you", "warm", "lithium", "slows", "down",
            "a", "river", "of", "ions", "every", "morning", "we", "kept", "going", "quietly",
        ];
        let (mut seed, mut out, mut n) = (7u64, String::new(), 0);
        let mut next = |m: u64| {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (seed >> 33) % m
        };
        let mut para = 0;
        while n < 20_000 {
            if para % 12 == 0 {
                out.push_str(&format!("# Chapter {}\n\n", para / 12 + 1));
            }
            for _ in 0..3 + next(5) {
                let len = 8 + next(14) as usize;
                for i in 0..len {
                    let w = W[next(W.len() as u64) as usize];
                    if i == 0 {
                        out.push_str(&w[..1].to_uppercase());
                        out.push_str(&w[1..]);
                    } else {
                        out.push(' ');
                        out.push_str(w);
                    }
                }
                out.push_str(". ");
                n += len;
            }
            out.push_str("\n\n");
            para += 1;
        }
        out
    }

    /// `cargo test -p booth-core --release bench_long -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn bench_long_document() {
        use crate::batch::{Job, JobLine, LineState};
        use std::time::Instant;
        let ms = |t: Instant| t.elapsed().as_secs_f64() * 1000.0;
        let best = |f: &dyn Fn()| (0..5).map(|_| { let t = Instant::now(); f(); ms(t) }).fold(f64::MAX, f64::min);

        let text = long_text();
        let parsed = parse(&text, &[]);
        println!("text: {} words, {} chars, {} lines", text.split_whitespace().count(), text.len(), parsed.lines.len());
        println!("parse: {:.1} ms", best(&|| { parse(&text, &[]); }));
        println!("open .md: {:.1} ms", best(&|| { open_bytes("a.md", text.as_bytes()).unwrap(); }));
        println!("open .txt: {:.1} ms", best(&|| { open_bytes("a.txt", text.as_bytes()).unwrap(); }));
        let body: String = text
            .split("\n\n")
            .map(|p| format!("<w:p><w:r><w:t>{p}</w:t></w:r></w:p>"))
            .collect();
        let docx_bytes = zip_of(&[("word/document.xml", &format!("<w:document><w:body>{body}</w:body></w:document>"))]);
        println!("open .docx: {:.1} ms", best(&|| { open_bytes("a.docx", &docx_bytes).unwrap(); }));
        let html = format!("<html><body>{}</body></html>", text.split("\n\n").map(|p| format!("<p>{p}</p>")).collect::<String>());
        println!("open .html: {:.1} ms", best(&|| { open_bytes("a.html", html.as_bytes()).unwrap(); }));

        // Saving progress after a line: script.json as it is now, and with the source in it (before).
        let job = Job {
            id: "x".into(),
            title: "x".into(),
            created_at: chrono::Utc::now(),
            lines: parsed
                .lines
                .iter()
                .enumerate()
                .map(|(i, l)| JobLine {
                    speaker: l.speaker.clone(),
                    voice: "rupert".into(),
                    voice_name: "Rupert".into(),
                    text: l.text.clone(),
                    para: l.para,
                    pause_ms: l.pause_ms,
                    state: LineState::Done,
                    seconds: 3.2,
                    seed: Some(1000 + i as u64),
                    version: 1,
                    voice_match: Some(0.91),
                    drifted: false,
                    error: None,
                    path: Some(format!(r"C:\Users\someone\AppData\Local\NoFaceTube\Booth\scripts\2026-11-17_101500_chapter-1\line-{i:04}-v1.wav").into()),
                })
                .collect(),
            skipped: parsed.skipped.clone(),
            source: text.clone(),
            voices: Default::default(),
            via: "app".into(),
            running: true,
            exported: None,
        };
        let now = serde_json::to_vec_pretty(&job).unwrap();
        let mut v = serde_json::to_value(&job).unwrap();
        v["source"] = serde_json::Value::String(text.clone());
        let before = serde_json::to_vec_pretty(&v).unwrap();
        let event = serde_json::to_vec(&job).unwrap();
        let dir = std::env::temp_dir().join("booth-bench");
        std::fs::create_dir_all(&dir).unwrap();
        let save = |bytes: &[u8]| {
            let tmp = dir.join("script.json.tmp");
            std::fs::write(&tmp, bytes).unwrap();
            std::fs::rename(&tmp, dir.join("script.json")).unwrap();
            let back: Job = serde_json::from_slice(&std::fs::read(dir.join("script.json")).unwrap()).unwrap();
            assert_eq!(back.lines.len(), job.lines.len());
        };
        println!(
            "script.json per line: {} KB now, {} KB before; read+write {:.1} ms now, {:.1} ms before",
            now.len() / 1024,
            before.len() / 1024,
            best(&|| save(&now)),
            best(&|| save(&before))
        );
        println!(
            "event to the UI per line: {} KB now ({:.1} ms to serialize), {} KB before",
            event.len() / 1024,
            best(&|| { serde_json::to_vec(&job).unwrap(); }),
            (event.len() + serde_json::to_vec(&text).unwrap().len()) / 1024
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
