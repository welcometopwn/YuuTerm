//! Lossless Tight and ZRLE for the negotiated 32-bit little-endian RGB888 format.
//! Zlib state belongs to a connection, never to a cloned framebuffer snapshot.
use super::*;
use flate2::{Decompress, FlushDecompress, Status};
const MAX_COMPRESSED: usize = 16 * 1024 * 1024;
const UPDATE_BUDGET: usize = 64 * 1024 * 1024;
pub(super) struct Decoder {
    tight: [Decompress; 4],
    zrle: Decompress,
    wire_bytes: usize,
    decoded_bytes: usize,
}
impl Default for Decoder {
    fn default() -> Self {
        Self {
            tight: std::array::from_fn(|_| Decompress::new(true)),
            zrle: Decompress::new(true),
            wire_bytes: 0,
            decoded_bytes: 0,
        }
    }
}
impl Decoder {
    pub fn begin_update(&mut self) {
        self.wire_bytes = 0;
        self.decoded_bytes = 0;
    }
    async fn blob<R: AsyncRead + Unpin>(&mut self, reader: &mut R, len: usize) -> Result<Vec<u8>> {
        anyhow::ensure!(
            len > 0 && len <= MAX_COMPRESSED && len <= UPDATE_BUDGET - self.wire_bytes,
            "VNC compressed input exceeds limit"
        );
        self.wire_bytes += len;
        let mut bytes = vec![0; len];
        reader.read_exact(&mut bytes).await?;
        Ok(bytes)
    }
    pub async fn tight<R: AsyncRead + Unpin>(
        &mut self,
        reader: &mut R,
        frame: &mut Framebuffer,
        [x, y, w, h]: [u16; 4],
    ) -> Result<()> {
        anyhow::ensure!(w <= 2048, "Tight rectangle width exceeds limit");
        let control = reader.read_u8().await?;
        for i in 0..4 {
            if control & (1 << i) != 0 {
                self.tight[i].reset(true);
            }
        }
        let mode = control >> 4;
        if mode == 8 {
            let mut rgb = [0; 3];
            reader.read_exact(&mut rgb).await?;
            for dy in 0..h {
                for dx in 0..w {
                    put(frame, x + dx, y + dy, rgb);
                }
            }
            return Ok(());
        }
        anyhow::ensure!(
            mode < 8,
            "Unadvertised JPEG or unsupported Tight compression method"
        );
        let filter = if mode & 4 != 0 {
            reader.read_u8().await?
        } else {
            0
        };
        anyhow::ensure!(filter <= 2, "Unknown Tight filter");
        let mut palette = Vec::new();
        if filter == 1 {
            let count = reader.read_u8().await? as usize + 1;
            anyhow::ensure!(count >= 2, "Invalid Tight palette size");
            for _ in 0..count {
                let mut rgb = [0; 3];
                reader.read_exact(&mut rgb).await?;
                palette.push(rgb);
            }
        }
        let row = if palette.len() == 2 {
            (w as usize).div_ceil(8)
        } else if !palette.is_empty() {
            w as usize
        } else {
            w as usize * 3
        };
        let expected = row * h as usize;
        anyhow::ensure!(
            expected <= UPDATE_BUDGET - self.decoded_bytes,
            "VNC decompression work exceeds limit"
        );
        let mut data = if expected < 12 {
            let mut data = vec![0; expected];
            reader.read_exact(&mut data).await?;
            data
        } else {
            let len = compact(reader).await?;
            let bytes = self.blob(reader, len).await?;
            let data = inflate(&mut self.tight[(mode & 3) as usize], &bytes, expected)?;
            anyhow::ensure!(data.len() == expected, "Tight decompressed length mismatch");
            data
        };
        self.decoded_bytes += data.len();
        if filter == 2 {
            for dy in 0..h as usize {
                for dx in 0..w as usize {
                    for component in 0..3 {
                        let at = (dy * w as usize + dx) * 3 + component;
                        let left = if dx > 0 { data[at - 3] as i32 } else { 0 };
                        let above = if dy > 0 {
                            data[at - w as usize * 3] as i32
                        } else {
                            0
                        };
                        let upper_left = if dx > 0 && dy > 0 {
                            data[at - w as usize * 3 - 3] as i32
                        } else {
                            0
                        };
                        data[at] =
                            data[at].wrapping_add((left + above - upper_left).clamp(0, 255) as u8);
                    }
                }
            }
        }
        for dy in 0..h as usize {
            for dx in 0..w as usize {
                let rgb = if palette.is_empty() {
                    let at = (dy * w as usize + dx) * 3;
                    [data[at], data[at + 1], data[at + 2]]
                } else {
                    let index = if palette.len() == 2 {
                        (data[dy * row + dx / 8] >> (7 - dx % 8)) as usize & 1
                    } else {
                        data[dy * row + dx] as usize
                    };
                    *palette
                        .get(index)
                        .context("Tight palette index outside palette")?
                };
                put(frame, x + dx as u16, y + dy as u16, rgb);
            }
        }
        Ok(())
    }
    pub async fn zrle<R: AsyncRead + Unpin>(
        &mut self,
        reader: &mut R,
        frame: &mut Framebuffer,
        [x, y, w, h]: [u16; 4],
    ) -> Result<()> {
        let len = reader.read_u32().await? as usize;
        let bytes = self.blob(reader, len).await?;
        // Plain RLE needs at most four bytes per pixel; allow the maximum
        // palette plus a type byte per tile, including skinny edge tiles.
        let tiles = (w as usize).div_ceil(64) * (h as usize).div_ceil(64);
        let bound =
            (w as usize * h as usize * 4 + tiles * 384).min(UPDATE_BUDGET - self.decoded_bytes);
        let data = inflate(&mut self.zrle, &bytes, bound)?;
        self.decoded_bytes += data.len();
        let mut input = Slice(&data);
        for ty in (0..h).step_by(64) {
            for tx in (0..w).step_by(64) {
                let tw = (w - tx).min(64) as usize;
                let th = (h - ty).min(64) as usize;
                let n = tw * th;
                let mode = input.byte()?;
                let mut palette = Vec::new();
                let palette_size = match mode {
                    1..=16 => mode as usize,
                    130..=255 => (mode - 128) as usize,
                    _ => 0,
                };
                for _ in 0..palette_size {
                    palette.push(input.pixel()?);
                }
                let mut pixels = Vec::with_capacity(n);
                match mode {
                    0 => {
                        for _ in 0..n {
                            pixels.push(input.pixel()?);
                        }
                    }
                    1 => pixels.resize(n, palette[0]),
                    2..=16 => {
                        let bits = if mode == 2 {
                            1
                        } else if mode <= 4 {
                            2
                        } else {
                            4
                        };
                        let per_byte = 8 / bits;
                        let row = tw.div_ceil(per_byte);
                        for _ in 0..th {
                            let packed = input.take(row)?;
                            for dx in 0..tw {
                                let index = (packed[dx / per_byte]
                                    >> (8 - bits - (dx % per_byte) * bits))
                                    as usize
                                    & ((1 << bits) - 1);
                                pixels.push(
                                    *palette
                                        .get(index)
                                        .context("ZRLE packed palette index outside palette")?,
                                );
                            }
                        }
                    }
                    128 | 130..=255 => {
                        while pixels.len() < n {
                            let (rgb, run) = if mode == 128 {
                                (input.pixel()?, true)
                            } else {
                                let index = input.byte()?;
                                (
                                    *palette
                                        .get((index & 127) as usize)
                                        .context("ZRLE run palette index outside palette")?,
                                    index & 128 != 0,
                                )
                            };
                            let count = if run { input.run(n - pixels.len())? } else { 1 };
                            pixels.resize(pixels.len() + count, rgb);
                        }
                    }
                    _ => bail!("Unsupported ZRLE tile subencoding"),
                }
                for (i, rgb) in pixels.into_iter().enumerate() {
                    put(
                        frame,
                        x + tx + (i % tw) as u16,
                        y + ty + (i / tw) as u16,
                        rgb,
                    );
                }
            }
        }
        anyhow::ensure!(input.0.is_empty(), "Unexpected bytes after ZRLE tiles");
        Ok(())
    }
}
fn put(frame: &mut Framebuffer, x: u16, y: u16, rgb: [u8; 3]) {
    let at = (y as usize * frame.width as usize + x as usize) * 4;
    frame.rgba[at..at + 4].copy_from_slice(&[rgb[0], rgb[1], rgb[2], 255]);
}
async fn compact<R: AsyncRead + Unpin>(reader: &mut R) -> Result<usize> {
    let a = reader.read_u8().await?;
    let mut value = (a & 127) as usize;
    if a & 128 != 0 {
        let b = reader.read_u8().await?;
        value |= ((b & 127) as usize) << 7;
        if b & 128 != 0 {
            value |= (reader.read_u8().await? as usize) << 14;
        }
    }
    Ok(value)
}
fn inflate(stream: &mut Decompress, input: &[u8], limit: usize) -> Result<Vec<u8>> {
    // A complete rectangle ends at a zlib SYNC_FLUSH boundary. Checking it
    // prevents accepting a shortened payload which already yielded its pixels.
    anyhow::ensure!(
        input.ends_with(&[0, 0, 255, 255]),
        "Truncated VNC zlib flush boundary"
    );
    anyhow::ensure!(
        limit <= UPDATE_BUDGET,
        "VNC decompression output exceeds limit"
    );
    let mut output = vec![0; limit + 1];
    let mut consumed = 0;
    let mut produced = 0;
    loop {
        let before_in = stream.total_in();
        let before_out = stream.total_out();
        let status = stream
            .decompress(
                &input[consumed..],
                &mut output[produced..],
                FlushDecompress::Sync,
            )
            .context("Invalid VNC zlib data")?;
        let read = (stream.total_in() - before_in) as usize;
        let written = (stream.total_out() - before_out) as usize;
        consumed += read;
        produced += written;
        anyhow::ensure!(produced <= limit, "VNC decompression output exceeds limit");
        anyhow::ensure!(
            status != Status::StreamEnd,
            "VNC persistent zlib stream ended unexpectedly"
        );
        if read == 0 && written == 0 {
            break;
        }
    }
    anyhow::ensure!(consumed == input.len(), "Unconsumed VNC compressed data");
    output.truncate(produced);
    Ok(output)
}
struct Slice<'a>(&'a [u8]);
impl<'a> Slice<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let value = self.0.get(..n).context("Truncated ZRLE tile")?;
        self.0 = &self.0[n..];
        Ok(value)
    }
    fn byte(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }
    fn pixel(&mut self) -> Result<[u8; 3]> {
        let b = self.take(3)?;
        Ok([b[2], b[1], b[0]])
    }
    fn run(&mut self, remaining: usize) -> Result<usize> {
        let mut count = 1;
        loop {
            let byte = self.byte()?;
            count += byte as usize;
            anyhow::ensure!(count <= remaining, "ZRLE run exceeds tile");
            if byte != 255 {
                return Ok(count);
            }
        }
    }
}

#[cfg(test)]
mod tests;
