//! QR transfer (S13): data carried from one screen to another camera, for
//! when no cable reaches. Light, not radio, so no event rule touches it.
//!
//! # A transfer
//!
//! Some bytes, of one [`Kind`], compressed with zlib and cut into frames.
//! Each frame is one QR code, and its text reads
//!
//! ```text
//! TT1:H:1A2B3C4D:2/5:<base45>
//! ```
//!
//! - `TT1`: this format. A frame from another version is refused by name.
//! - `H`: the kind, one letter.
//! - `1A2B3C4D`: the transfer's id, the CRC-32 of all of its compressed
//!   bytes, so frames of two transfers never mix, and the whole is checked
//!   once it is together. zlib's own checksum checks it again.
//! - `2/5`: this frame's place, counting from 1, and how many there are.
//! - The frame's share of the compressed bytes, in base45 ([RFC 9285]).
//!
//! Every character is in QR's alphanumeric set (digits, capitals, and
//! ` $%*+-./:`), which packs 5.5 bits per character where byte mode packs
//! 8 per byte: base45's 1.5 characters a byte costs 8.25 bits. A browser's
//! detector hands back the text as it is, which raw bytes would not survive.
//!
//! # Many frames, shown in turn
//!
//! The sender shows its frames one after another and starts again, for as
//! long as it is asked to. The receiver keeps every frame it has not seen
//! and stops when it has them all, in any order. A frame the camera missed
//! comes round again. The plan's fountain code would spare the wait for
//! that one frame; it suggested plain frames first, and a scout's handful
//! of forms is two or three frames.
//!
//! The browser has the same format in `static/js/qr.js`: a tablet with no
//! Pi frames its own forms, and reads what the Pi shows back.
//!
//! [RFC 9285]: https://www.rfc-editor.org/rfc/rfc9285

use std::fmt::Write as _;

use qrcodegen::{QrCode, QrCodeEcc};
use thiserror::Error;

/// What every frame of this format begins with.
pub const PREFIX: &str = "TT1:";

/// Compressed bytes per frame. 400 bytes are 600 base45 characters, which
/// with the header is a version 17 code at medium error correction: 85
/// modules a side, which a phone reads off a tablet at arm's length.
pub const FRAME_BYTES: usize = 400;

/// The most frames one transfer may have: about 40 KB compressed.
pub const MAX_FRAMES: usize = 99;

/// The most a transfer may hold once inflated.
pub const MAX_PAYLOAD: usize = 256 * 1024;

/// Modules of blank border round a code, as the standard asks.
pub const QUIET_ZONE: usize = 4;

/// What a transfer carries, which says how to read it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A tablet's unsent scouting forms and its scout's token, for a lead
    /// scout to record: a [`Handoff`] as JSON.
    Handoff,
    /// What the Pi made of a handoff, for the tablet to read back: a
    /// [`HandoffReceipts`] as JSON.
    Receipt,
}

impl Kind {
    pub fn letter(self) -> char {
        match self {
            Kind::Handoff => 'H',
            Kind::Receipt => 'R',
        }
    }

    pub fn from_letter(letter: &str) -> Option<Kind> {
        match letter {
            "H" => Some(Kind::Handoff),
            "R" => Some(Kind::Receipt),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum QrError {
    #[error("no code has been read yet")]
    Nothing,
    #[error("this is not a TealTeam code")]
    NotOurs,
    #[error("this code was made by another version of TealTeam ({0})")]
    OtherVersion(String),
    #[error("this code is damaged: {0}")]
    Malformed(&'static str),
    #[error("this code belongs to another transfer")]
    OtherTransfer,
    #[error("{missing} of {count} codes are still missing")]
    Incomplete { missing: usize, count: usize },
    #[error("the codes do not add up: {0}")]
    Corrupt(&'static str),
    #[error("too much for one transfer: at most {MAX_FRAMES} codes")]
    TooLarge,
}

/// The frames' texts for `payload`, in order.
pub fn frames(kind: Kind, payload: &[u8]) -> Result<Vec<String>, QrError> {
    if payload.len() > MAX_PAYLOAD {
        return Err(QrError::TooLarge);
    }
    let packed = miniz_oxide::deflate::compress_to_vec_zlib(payload, 9);
    let count = packed.len().div_ceil(FRAME_BYTES).max(1);
    if count > MAX_FRAMES {
        return Err(QrError::TooLarge);
    }
    let id = crc32(&packed);
    Ok(packed
        .chunks(FRAME_BYTES)
        .enumerate()
        .map(|(i, chunk)| {
            format!(
                "{PREFIX}{}:{id:08X}:{}/{count}:{}",
                kind.letter(),
                i + 1,
                base45_encode(chunk)
            )
        })
        .collect())
}

/// One frame, read but not yet decoded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame<'a> {
    pub kind: Kind,
    pub id: u32,
    /// Counting from 1.
    pub index: usize,
    pub count: usize,
    pub data: &'a str,
}

/// Read a frame's header. Its data is decoded when it is added.
pub fn parse(text: &str) -> Result<Frame<'_>, QrError> {
    let text = text.trim();
    let Some(rest) = text.strip_prefix(PREFIX) else {
        return match text.split_once(':') {
            Some((tag, _))
                if tag.len() > 2
                    && tag.starts_with("TT")
                    && tag[2..].bytes().all(|b| b.is_ascii_digit()) =>
            {
                Err(QrError::OtherVersion(tag.to_string()))
            }
            _ => Err(QrError::NotOurs),
        };
    };
    let mut parts = rest.splitn(4, ':');
    let (Some(kind), Some(id), Some(place), Some(data)) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return Err(QrError::Malformed("a part of its header is missing"));
    };
    let kind = Kind::from_letter(kind).ok_or(QrError::Malformed("an unknown kind"))?;
    if id.len() != 8 {
        return Err(QrError::Malformed("its id is not 8 digits"));
    }
    let id = u32::from_str_radix(id, 16).map_err(|_| QrError::Malformed("its id is not hex"))?;
    let (index, count) = place
        .split_once('/')
        .and_then(|(i, n)| Some((i.parse::<usize>().ok()?, n.parse::<usize>().ok()?)))
        .ok_or(QrError::Malformed("its place is not i/n"))?;
    if count == 0 || count > MAX_FRAMES || index == 0 || index > count {
        return Err(QrError::Malformed("its place is out of range"));
    }
    Ok(Frame {
        kind,
        id,
        index,
        count,
        data,
    })
}

/// How far a transfer has come.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Progress {
    pub kind: Kind,
    pub have: usize,
    pub count: usize,
}

impl Progress {
    pub fn done(&self) -> bool {
        self.have == self.count
    }
}

/// Collects one transfer's frames, in any order, until it has them all.
#[derive(Debug, Default)]
pub struct Assembler {
    current: Option<Pending>,
}

#[derive(Debug)]
struct Pending {
    kind: Kind,
    id: u32,
    chunks: Vec<Option<Vec<u8>>>,
}

impl Assembler {
    pub fn new() -> Self {
        Self::default()
    }

    /// Take one frame's text. A frame seen before changes nothing. One of
    /// another transfer is refused with [`QrError::OtherTransfer`]: the
    /// caller decides whether to [`Assembler::reset`] and start on it.
    pub fn add(&mut self, text: &str) -> Result<Progress, QrError> {
        let frame = parse(text)?;
        let pending = self.current.get_or_insert_with(|| Pending {
            kind: frame.kind,
            id: frame.id,
            chunks: vec![None; frame.count],
        });
        if pending.id != frame.id || pending.kind != frame.kind {
            return Err(QrError::OtherTransfer);
        }
        if pending.chunks.len() != frame.count {
            return Err(QrError::Corrupt("two codes disagree on how many there are"));
        }
        let slot = &mut pending.chunks[frame.index - 1];
        if slot.is_none() {
            let bytes =
                base45_decode(frame.data).ok_or(QrError::Malformed("its data is not base45"))?;
            // Every frame but the last is full.
            let full = bytes.len() == FRAME_BYTES;
            let last = frame.index == frame.count;
            if bytes.is_empty() || bytes.len() > FRAME_BYTES || (!last && !full) {
                return Err(QrError::Malformed("its data is the wrong length"));
            }
            *slot = Some(bytes);
        }
        Ok(self.progress().expect("a transfer was started"))
    }

    pub fn progress(&self) -> Option<Progress> {
        self.current.as_ref().map(|p| Progress {
            kind: p.kind,
            have: p.chunks.iter().filter(|c| c.is_some()).count(),
            count: p.chunks.len(),
        })
    }

    /// Which frames are still missing, counting from 1.
    pub fn missing(&self) -> Vec<usize> {
        self.current.as_ref().map_or_else(Vec::new, |p| {
            (1..=p.chunks.len())
                .filter(|i| p.chunks[i - 1].is_none())
                .collect()
        })
    }

    pub fn reset(&mut self) {
        self.current = None;
    }

    /// The transfer's kind and bytes, once every frame is in.
    pub fn finish(&self) -> Result<(Kind, Vec<u8>), QrError> {
        let pending = self.current.as_ref().ok_or(QrError::Nothing)?;
        let missing = pending.chunks.iter().filter(|c| c.is_none()).count();
        if missing > 0 {
            return Err(QrError::Incomplete {
                missing,
                count: pending.chunks.len(),
            });
        }
        let packed: Vec<u8> = pending.chunks.iter().flatten().flatten().copied().collect();
        if crc32(&packed) != pending.id {
            return Err(QrError::Corrupt("their checksum does not match"));
        }
        let payload = miniz_oxide::inflate::decompress_to_vec_zlib_with_limit(&packed, MAX_PAYLOAD)
            .map_err(|_| QrError::Corrupt("they do not inflate"))?;
        Ok((pending.kind, payload))
    }
}

/// A whole transfer from its frames' texts, in any order, repeats allowed.
pub fn assemble<'a>(texts: impl IntoIterator<Item = &'a str>) -> Result<(Kind, Vec<u8>), QrError> {
    let mut assembler = Assembler::new();
    for text in texts {
        assembler.add(text)?;
    }
    assembler.finish()
}

/// A frame drawn as a QR code: `size` modules a side, quiet zone included,
/// and the dark modules as SVG path data in those units, for a template to
/// put in `<path d="...">` with `viewBox="0 0 size size"`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Symbol {
    pub size: usize,
    pub path: String,
}

/// Draw `text` at medium error correction, or higher where it fits the same
/// size. Medium survives glare on a tablet's glass better than low.
pub fn symbol(text: &str) -> Result<Symbol, QrError> {
    let code = QrCode::encode_text(text, QrCodeEcc::Medium).map_err(|_| QrError::TooLarge)?;
    let modules = usize::try_from(code.size()).expect("a QR code's size is positive");
    let mut path = String::new();
    for y in 0..modules {
        let mut x = 0;
        while x < modules {
            let dark = |x: usize| code.get_module(x as i32, y as i32);
            if !dark(x) {
                x += 1;
                continue;
            }
            let start = x;
            while x < modules && dark(x) {
                x += 1;
            }
            // One rectangle per run of dark modules in a row.
            let _ = write!(
                path,
                "M{},{}h{}v1h-{}z",
                start + QUIET_ZONE,
                y + QUIET_ZONE,
                x - start,
                x - start
            );
        }
    }
    Ok(Symbol {
        size: modules + 2 * QUIET_ZONE,
        path,
    })
}

// ── What travels ────────────────────────────────────────────────────────────

/// A tablet's forms that never reached the Pi, carried to a lead scout who
/// can reach it ([`Kind::Handoff`]). `static/js/handoff.js` writes it from
/// what `draft.js` kept (C3).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Handoff {
    /// The scout's offline token (C9). It says who saved the forms and on
    /// which tablet, and the Pi checks its signature, so a lead records
    /// them as that scout, not as themself.
    pub token: String,
    pub forms: Vec<HandedForm>,
}

/// One form as the tablet kept it: answers by input name, as the form
/// would post them, a ticked box as `true`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct HandedForm {
    pub record_id: String,
    #[serde(rename = "match")]
    pub match_key: String,
    pub team: String,
    /// The version of the form it was typed into.
    pub form_version: i64,
    /// When the tablet last kept it, in Unix milliseconds on its clock.
    pub saved_at: i64,
    pub answers: std::collections::BTreeMap<String, serde_json::Value>,
}

impl HandedForm {
    /// The answers as a form post's pairs: a ticked box as `on`, anything
    /// that is neither text nor `true` left out.
    pub fn pairs(&self) -> Vec<(String, String)> {
        self.answers
            .iter()
            .filter_map(|(name, value)| match value {
                serde_json::Value::String(text) => Some((name.clone(), text.clone())),
                serde_json::Value::Bool(true) => Some((name.clone(), "on".into())),
                _ => None,
            })
            .collect()
    }
}

/// The Pi's answer to a handoff ([`Kind::Receipt`]), shown as a code for
/// the tablet to read, so it knows which forms it can let go of.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct HandoffReceipts {
    pub receipts: Vec<crate::outbox::Receipt>,
}

// ── Encodings ───────────────────────────────────────────────────────────────

const BASE45: &[u8; 45] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ $%*+-./:";

/// RFC 9285: each two bytes as three characters, a last odd byte as two.
pub fn base45_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(2) * 3);
    let digit = |n: usize| char::from(BASE45[n % 45]);
    for pair in bytes.chunks(2) {
        if let [a, b] = pair {
            let n = usize::from(*a) * 256 + usize::from(*b);
            out.extend([digit(n), digit(n / 45), digit(n / 2025)]);
        } else {
            let n = usize::from(pair[0]);
            out.extend([digit(n), digit(n / 45)]);
        }
    }
    out
}

pub fn base45_decode(text: &str) -> Option<Vec<u8>> {
    let values: Vec<u32> = text
        .bytes()
        .map(|c| BASE45.iter().position(|&d| d == c).map(|v| v as u32))
        .collect::<Option<_>>()?;
    let mut out = Vec::with_capacity(values.len() / 3 * 2 + 1);
    for group in values.chunks(3) {
        match group {
            [c, d, e] => {
                let n = c + d * 45 + e * 2025;
                if n > 0xFFFF {
                    return None;
                }
                out.extend([(n >> 8) as u8, n as u8]);
            }
            [c, d] => {
                let n = c + d * 45;
                if n > 0xFF {
                    return None;
                }
                out.push(n as u8);
            }
            _ => return None,
        }
    }
    Some(out)
}

/// CRC-32 (IEEE), as zip and PNG have it. Bitwise: a transfer is a few KB.
pub fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &byte in bytes {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xEDB8_8320 & (crc & 1).wrapping_neg());
        }
    }
    !crc
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Bytes zlib cannot shrink, so the frame count is predictable.
    fn noise(len: usize) -> Vec<u8> {
        let mut state = 0x2545_F491_u32;
        (0..len)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 17;
                state ^= state << 5;
                state as u8
            })
            .collect()
    }

    #[test]
    fn base45_matches_the_rfc() {
        // RFC 9285, section 4.
        assert_eq!(base45_encode(b"AB"), "BB8");
        assert_eq!(base45_encode(b"Hello!!"), "%69 VD92EX0");
        assert_eq!(base45_encode(b"base-45"), "UJCLQE7W581");
        assert_eq!(base45_decode("QED8WEX0").unwrap(), b"ietf!");
        assert_eq!(base45_decode("").unwrap(), b"");
        // Out of range, a lone character, and a letter outside the set.
        assert_eq!(base45_decode("GGW"), None);
        assert_eq!(base45_decode("BB8A"), None);
        assert_eq!(base45_decode("bb8"), None);
        let all: Vec<u8> = (0..=255).collect();
        assert_eq!(base45_decode(&base45_encode(&all)).unwrap(), all);
    }

    #[test]
    fn crc32_is_the_ieee_one() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(crc32(b""), 0);
    }

    #[test]
    fn a_small_transfer_is_one_frame_of_alphanumerics() {
        let frames = frames(Kind::Handoff, br#"{"forms":[]}"#).unwrap();
        assert_eq!(frames.len(), 1);
        let frame = parse(&frames[0]).unwrap();
        assert_eq!(
            (frame.kind, frame.index, frame.count),
            (Kind::Handoff, 1, 1)
        );
        assert!(frames[0].starts_with("TT1:H:"));
        assert!(
            frames[0].bytes().all(|c| BASE45.contains(&c)),
            "{}",
            frames[0]
        );
        assert_eq!(
            assemble(frames.iter().map(String::as_str)).unwrap(),
            (Kind::Handoff, br#"{"forms":[]}"#.to_vec())
        );
    }

    #[test]
    fn frames_come_together_in_any_order_with_repeats() {
        let payload = noise(FRAME_BYTES * 3 + 17);
        let frames = frames(Kind::Receipt, &payload).unwrap();
        assert_eq!(frames.len(), 4, "zlib adds a few bytes to noise");

        let mut assembler = Assembler::new();
        for (i, at) in [2, 0, 2, 3].into_iter().enumerate() {
            let progress = assembler.add(&frames[at]).unwrap();
            assert_eq!(progress.count, 4);
            assert_eq!(progress.have, [1, 2, 2, 3][i]);
            assert!(!progress.done());
        }
        assert_eq!(assembler.missing(), vec![2]);
        assert_eq!(
            assembler.finish(),
            Err(QrError::Incomplete {
                missing: 1,
                count: 4
            })
        );
        assert!(assembler.add(&frames[1]).unwrap().done());
        assert_eq!(assembler.finish().unwrap(), (Kind::Receipt, payload));
    }

    #[test]
    fn another_transfer_is_refused_until_reset() {
        let first = frames(Kind::Handoff, &noise(900)).unwrap();
        let second = frames(Kind::Handoff, &noise(901)).unwrap();
        let mut assembler = Assembler::new();
        assembler.add(&first[0]).unwrap();
        assert_eq!(assembler.add(&second[0]), Err(QrError::OtherTransfer));
        assembler.reset();
        assert_eq!(assembler.add(&second[0]).unwrap().have, 1);
    }

    #[test]
    fn foreign_and_damaged_codes_say_what_they_are() {
        assert_eq!(parse("https://example.com"), Err(QrError::NotOurs));
        assert_eq!(parse("WIFI:S:x;;"), Err(QrError::NotOurs));
        assert_eq!(
            parse("TT2:H:00000000:1/1:00"),
            Err(QrError::OtherVersion("TT2".into()))
        );
        for bad in [
            "TT1:H:00000000:1/1",
            "TT1:X:00000000:1/1:00",
            "TT1:H:0000:1/1:00",
            "TT1:H:0000000G:1/1:00",
            "TT1:H:00000000:2/1:00",
            "TT1:H:00000000:0/1:00",
            "TT1:H:00000000:1/100:00",
            "TT1:H:00000000:one/1:00",
        ] {
            assert!(matches!(parse(bad), Err(QrError::Malformed(_))), "{bad}");
        }
        // A short frame that is not the last is damaged.
        let frames = frames(Kind::Handoff, &noise(900)).unwrap();
        let short = frames[0].replacen(&frames[0][frames[0].len() - 3..], "", 1);
        assert!(matches!(
            Assembler::new().add(&short),
            Err(QrError::Malformed(_))
        ));
    }

    #[test]
    fn a_changed_byte_is_caught() {
        let mut frames = frames(Kind::Handoff, b"some forms").unwrap();
        let header = frames[0].rsplit_once(':').unwrap().0.to_string();
        let data = frames[0].rsplit_once(':').unwrap().1;
        let mut bytes = base45_decode(data).unwrap();
        bytes[3] ^= 1;
        frames[0] = format!("{header}:{}", base45_encode(&bytes));
        assert_eq!(
            assemble(frames.iter().map(String::as_str)),
            Err(QrError::Corrupt("their checksum does not match"))
        );
    }

    #[test]
    fn a_transfer_has_limits() {
        assert_eq!(
            frames(Kind::Handoff, &noise(MAX_PAYLOAD + 1)),
            Err(QrError::TooLarge)
        );
        assert_eq!(
            frames(Kind::Handoff, &noise(FRAME_BYTES * MAX_FRAMES)),
            Err(QrError::TooLarge)
        );
        assert_eq!(
            frames(Kind::Handoff, &noise(FRAME_BYTES * (MAX_FRAMES - 1)))
                .unwrap()
                .len(),
            MAX_FRAMES
        );
    }

    /// Paint a symbol's path onto a pixel grid and read it back with a
    /// decoder that shares no code with the encoder.
    fn read_back(symbol: &Symbol) -> String {
        const SCALE: usize = 3;
        let side = symbol.size * SCALE;
        let mut dark = vec![false; symbol.size * symbol.size];
        for rect in symbol.path.split('z').filter(|r| !r.is_empty()) {
            let (at, rest) = rect.trim_start_matches('M').split_once('h').unwrap();
            let (x, y) = at.split_once(',').unwrap();
            let (x, y): (usize, usize) = (x.parse().unwrap(), y.parse().unwrap());
            let run: usize = rest.split_once('v').unwrap().0.parse().unwrap();
            for dx in 0..run {
                dark[y * symbol.size + x + dx] = true;
            }
        }
        let mut image = rqrr::PreparedImage::prepare_from_greyscale(side, side, |x, y| {
            if dark[(y / SCALE) * symbol.size + x / SCALE] {
                0
            } else {
                255
            }
        });
        let grids = image.detect_grids();
        assert_eq!(grids.len(), 1, "one code found");
        grids[0].decode().unwrap().1
    }

    #[test]
    fn a_full_frame_draws_as_a_code_that_reads_back() {
        let frames = frames(Kind::Handoff, &noise(FRAME_BYTES * 2)).unwrap();
        let first = symbol(&frames[0]).unwrap();
        // Version 17 is 85 modules, version 18 is 89.
        assert!(first.size - 2 * QUIET_ZONE <= 89, "{}", first.size);
        assert_eq!(read_back(&first), frames[0]);
        let last = symbol(&frames[2]).unwrap();
        assert_eq!(read_back(&last), frames[2]);
    }

    #[test]
    fn a_handed_form_posts_as_the_form_would() {
        let form: HandedForm = serde_json::from_str(
            r#"{"record_id":"r","match":"2026now_qm1","team":"254","form_version":3,
                "saved_at":1,"answers":{"f.auto":"3","f.climbed":true,"f.broke":false,"f.n":null}}"#,
        )
        .unwrap();
        assert_eq!(
            form.pairs(),
            vec![
                ("f.auto".to_string(), "3".to_string()),
                ("f.climbed".to_string(), "on".to_string())
            ]
        );
    }
}
