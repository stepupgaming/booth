# Changelog

What changed in each Booth release. The newest is at the top.

This file is the one source for release notes. The release workflow copies a version's section
into its GitHub Release and into the update notice the app shows, and nofacetube.com/tools/booth/changelog/
shows the whole file. Write for people who use Booth, not for developers: say what they can do now
or what stopped going wrong.

Each version starts with a `## <version> (<date>)` heading. A release candidate such as
`0.4.1-rc.2` uses the `## 0.4.1` section.

## 0.5.0 (2026-11-17)

- **AMD and Intel graphics cards (beta).** On Windows and Linux, Settings > Run on has
  "AMD or Intel card". Booth downloads a separate graphics engine for it, so the app itself
  stays the same size. It's new and we don't have those cards, so please tell us how it went:
  nofacetube.com/tools/booth/gpu-beta/
- **Pronunciation list.** Settings > Pronunciation: tell Booth how to say a name, a brand or an
  acronym once ("SQL" as "sequel"), and every line, script and Claude request uses it. Your
  scripts keep the words as you typed them.
- **Open a document.** Script opens Word (.docx), PDF, EPUB, Markdown, HTML, RTF, subtitles
  (.srt, .vtt) and Fountain screenplays as well as .txt. On a computer you can drop the file
  onto Booth. Long scripts, such as a whole book chapter, stay quick to scroll and edit.
- **Pauses.** Type `[pause]` or `[pause 2s]` where you want silence. Booth never reads it aloud.
- **Export, ready to post.** Save the voiceover as MP3 as well as WAV. "Even out loudness"
  brings it to YouTube's level (-14 LUFS) with no clipping. Captions can be short, 2 to 4 words
  at a time for Shorts, TikTok and Reels, and come as .vtt too.
- **Recording a voice in your language.** Create a voice has a passage to read in each of the
  43 languages, not only English.
- A newer engine from Zyphra (ZONOS3-Mini 96d9e0e): numbers, dates, times and prices are read
  properly in 34 languages ("2,5" in French is "deux virgule cinq").

### Fixed

- Android: recording a voice never asked for the microphone. Booth now asks the first time you
  press Record.
- A voice you were making was lost when you switched tabs. It now waits for you, and Start over
  clears it.
- The transcript field in Create a voice is gone. Cloning never used it, so it only got in the
  way.

## 0.4.2 (2026-11-15)

- Fixed on Mac: recording a voice failed with "Couldn't use the microphone" and macOS never
  asked. Booth now asks for the microphone the first time you press Record.
- If the microphone is turned off for Booth, the message now says where to turn it back on.

## 0.4.1 (2026-11-15)

- **Script.** A new tab for whole scripts. Paste a script (or open a .txt) and Booth splits it
  into lines, finds the speakers (`NARRATOR:`, `GUEST:`) and leaves out notes such as
  `[B-roll: ...]` and `(SFX: ...)`, so they are never read aloud. Pick a voice for each speaker and
  press Make: Booth makes the lines one after another while you do something else.
- **Captions for free.** Export gives you the whole voiceover as one file plus a captions file
  (.srt) timed to it, ready for CapCut, Premiere or YouTube. You can also get every line as its
  own file, and choose the pause between lines and between paragraphs.
- **Fix one line.** Tap a line to change its words or redo it. Only that line is made again.
- **On phones,** a script keeps going with the screen off, and Share sends the voiceover and
  captions straight to CapCut, Drive or a chat. The notification shows which line it's on.
- Phones: the Back button now sends Booth to the background instead of closing it, so a script
  being made keeps going. The tab bar steps aside while the keyboard is up, and voices no longer
  take minutes to appear when Booth was closed while starting.
- **For AI assistants:** booth-mcp has `make_script`, `script_status` and `redo_script_line`, so
  Claude can voice a whole script and hand back the voiceover and captions.
- Fixed: cloning failed with "could not decode the reference audio" on computers without
  ffmpeg. The desktop apps now bring their own decoder.
- Creating a voice has a Language picker and a sample line in that language, so a Spanish
  voice can be tried in Spanish.
- Every release is now installed and tested on fresh Windows, Linux and Mac machines.

## 0.4.0 (2026-11-15)

- Takes per line is a 1 to 10 stepper next to Generate, instead of a setting.
- Delivery and Shape fold away (closed by default on phones) and show how many controls you
  changed, with a Reset.
- Steadiness: takes that sound more alike from one to the next.
- Drift check: in Exact mode, a take that sounds like someone else is made again with a new
  seed, up to twice, and the closest one is kept.
- On phones, tap a line to pick it and tap again to edit it, so moving between lines to hear
  takes no longer opens the keyboard. Long lines wrap instead of being cut off.

## 0.3.2 (2026-11-13)

- The Android APK is built and signed by the release workflow, so later versions install over
  the old one and keep your takes.

## 0.3.1 (2026-11-13)

- booth-mcp frees the engine when Claude closes it, and after ten quiet minutes, so it no
  longer holds about 700 MB (and the graphics card) while Claude stays open.

## 0.3.0 (2026-11-13)

- Booth runs on Android phones and tablets (arm64, Android 7 and later), on the processor.
- Renders keep going while you are in another app on Android.
- A new Library tab with every take you have made.
- Setup knows when it is on a phone and never needs scrolling.
- booth.log records engine starts, restarts and every render, for bug reports.
- Booth has its own icon on every platform.
- Fixed: opening a voice from the Library crashed the app.
- Fixed: setup on a phone said "1-core processor".

## 0.2.3 (2026-11-13)

- All 43 languages: a language picker in the studio, with English (US) first.
- A newer engine from Zyphra (ZONOS3-Mini b171b19).
- Fixed: NVIDIA engine upgrades were never found.

## 0.2.2 (2026-11-13)

- A newer engine from Zyphra (ZONOS3-Mini bcb6661), with steering that holds better.
- NVIDIA installs move to the new engine in the background.

## 0.2.1 (2026-11-13)

- Model files come from a pinned list, from Hugging Face first and a GitHub copy if that fails,
  so a download that breaks halfway carries on from the other source.

## 0.2.0 (2026-11-12)

- Booth runs on the processor, so it works without an NVIDIA card.
- Booth updates itself.
- First Mac (Apple Silicon) and Linux versions.

## 0.1.1 (2026-11-12)

- Styled dropdowns and a real app icon.
- Only one engine runs at a time.

## 0.1.0 (2026-11-12)

- First release: free text to speech on your own Windows PC, on ZONOS3-Mini.
