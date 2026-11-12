//! Embedded third-party assets (see assets/CREDITS.md): textures, models and sounds.
include!(concat!(env!("OUT_DIR"), "/assets_table.rs"));

pub fn get(path: &str) -> Option<&'static [u8]> {
    FILES.iter().find(|f| f.0 == path).map(|f| f.1)
}

/// All embedded paths starting with `prefix`, sorted.
pub fn list(prefix: &str) -> Vec<&'static str> {
    FILES.iter().filter(|f| f.0.starts_with(prefix)).map(|f| f.0).collect()
}

pub struct Image {
    pub w: usize,
    pub h: usize,
    /// RGBA8
    pub px: Vec<u8>,
}

pub fn decode_png(bytes: &[u8]) -> Option<Image> {
    let mut dec = png::Decoder::new(std::io::Cursor::new(bytes));
    dec.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut r = dec.read_info().ok()?;
    let mut buf = vec![0; r.output_buffer_size()?];
    let info = r.next_frame(&mut buf).ok()?;
    let (w, h) = (info.width as usize, info.height as usize);
    let src = &buf[..info.buffer_size()];
    let px = match info.color_type {
        png::ColorType::Rgba => src.to_vec(),
        png::ColorType::Rgb => src.chunks(3).flat_map(|c| [c[0], c[1], c[2], 255]).collect(),
        png::ColorType::GrayscaleAlpha => src.chunks(2).flat_map(|c| [c[0], c[0], c[0], c[1]]).collect(),
        png::ColorType::Grayscale => src.iter().flat_map(|&c| [c, c, c, 255]).collect(),
        png::ColorType::Indexed => return None,
    };
    Some(Image { w, h, px })
}

pub fn image(path: &str) -> Option<Image> {
    decode_png(get(path)?)
}

/// Decode a 16-bit PCM WAV into mono f32 samples and its sample rate.
pub fn decode_wav(bytes: &[u8]) -> Option<(Vec<f32>, u32)> {
    if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return None;
    }
    let (mut i, mut channels, mut rate, mut bits) = (12usize, 1u16, 44100u32, 16u16);
    while i + 8 <= bytes.len() {
        let id = &bytes[i..i + 4];
        let len = u32::from_le_bytes(bytes[i + 4..i + 8].try_into().ok()?) as usize;
        let body = &bytes[i + 8..(i + 8 + len).min(bytes.len())];
        if id == b"fmt " {
            channels = u16::from_le_bytes(body[2..4].try_into().ok()?);
            rate = u32::from_le_bytes(body[4..8].try_into().ok()?);
            bits = u16::from_le_bytes(body[14..16].try_into().ok()?);
        } else if id == b"data" {
            if bits != 16 {
                return None;
            }
            let ch = channels.max(1) as usize;
            let frames = body.len() / 2 / ch;
            let mut out = Vec::with_capacity(frames);
            for f in 0..frames {
                let mut acc = 0.0;
                for c in 0..ch {
                    let o = (f * ch + c) * 2;
                    acc += i16::from_le_bytes([body[o], body[o + 1]]) as f32 / 32768.0;
                }
                out.push(acc / ch as f32);
            }
            return Some((out, rate));
        }
        i += 8 + len + (len & 1);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn embedded_assets_decode() {
        assert!(FILES.len() > 100, "assets missing: {}", FILES.len());
        for p in list("textures/") {
            let im = image(p).unwrap_or_else(|| panic!("bad png {p}"));
            assert_eq!((im.w, im.h), (256, 256), "{p}");
        }
        assert!(image("models/atlas.png").is_some());
        for p in list("audio/") {
            let (s, r) = decode_wav(get(p).unwrap()).unwrap_or_else(|| panic!("bad wav {p}"));
            assert!(r == 32000 && !s.is_empty(), "{p}");
        }
        for p in list("models/").into_iter().filter(|p| p.ends_with(".glb")) {
            gltf::Gltf::from_slice(get(p).unwrap()).unwrap_or_else(|e| panic!("bad glb {p}: {e}"));
        }
    }
}
