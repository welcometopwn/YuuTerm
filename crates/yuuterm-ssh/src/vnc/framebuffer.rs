use super::*;

pub(super) const MAX_PIXELS: usize = 4 * 1024 * 1024;
const MAX_RECTANGLES: u16 = 4096;
const MAX_UPDATE_BYTES: usize = 64 * 1024 * 1024;
/// Larger shapes are consumed but not shown; WebView2 ignores CSS cursors over 128 px.
const MAX_CURSOR: u16 = 256;
const SHOWN_CURSOR: u16 = 128;
/// Packed RGBA8, alpha always 255. No server clipboard data is exposed.
#[derive(Clone, Debug)]
pub struct Framebuffer {
    pub width: u16,
    pub height: u16,
    pub rgba: Vec<u8>,
    pub resize: Option<Screen>,
    pub resize_status: Option<u16>,
    /// Cursor pseudo-encoding: the server stops drawing its pointer into the
    /// framebuffer. `None` until the server sends a shape; zero size = hidden.
    pub cursor: Option<Arc<Cursor>>,
}
/// Packed RGBA8 with the hotspot inside the shape.
#[derive(Debug)]
pub struct Cursor {
    pub hotspot_x: u16,
    pub hotspot_y: u16,
    pub width: u16,
    pub height: u16,
    pub rgba: Vec<u8>,
}
#[derive(Clone, Copy, Debug)]
pub struct Screen {
    pub id: u32,
    pub flags: u32,
}
impl Framebuffer {
    pub(super) fn new(width: u16, height: u16) -> Result<Self> {
        let pixels = width as usize * height as usize;
        anyhow::ensure!(
            width > 0 && height > 0 && pixels <= MAX_PIXELS,
            "VNC framebuffer dimensions exceed limit"
        );
        Ok(Self {
            width,
            height,
            rgba: [0, 0, 0, 255].repeat(pixels),
            resize: None,
            resize_status: None,
            cursor: None,
        })
    }
    fn rectangle(&self, x: u16, y: u16, w: u16, h: u16) -> Result<()> {
        anyhow::ensure!(
            w > 0
                && h > 0
                && u32::from(x) + u32::from(w) <= u32::from(self.width)
                && u32::from(y) + u32::from(h) <= u32::from(self.height),
            "VNC rectangle outside framebuffer"
        );
        Ok(())
    }
    #[cfg(test)]
    pub(super) async fn update<R: AsyncRead + Unpin>(&mut self, reader: &mut R) -> Result<()> {
        self.update_with(reader, &mut compression::Decoder::default())
            .await
    }
    #[cfg(test)]
    pub(super) async fn update_with<R: AsyncRead + Unpin>(
        &mut self,
        reader: &mut R,
        decoder: &mut compression::Decoder,
    ) -> Result<()> {
        let kind = reader.read_u8().await?;
        self.message(kind, reader, decoder).await?;
        Ok(())
    }
    pub(super) async fn message<R: AsyncRead + Unpin>(
        &mut self,
        kind: u8,
        reader: &mut R,
        decoder: &mut compression::Decoder,
    ) -> Result<bool> {
        if kind == 2 {
            return Ok(false);
        } // Bell: no automatic desktop action.
        if kind == 3 {
            let mut padding = [0; 3];
            reader.read_exact(&mut padding).await?;
            let length = reader.read_u32().await? as usize;
            anyhow::ensure!(
                length <= MAX_CLIPBOARD_BYTES,
                "VNC server clipboard exceeds limit"
            );
            // Unsolicited clipboard is consumed and discarded, never exposed to
            // the UI, logs or Windows clipboard. No extended clipboard requested.
            let mut chunk = [0; 1024];
            let mut remaining = length;
            while remaining > 0 {
                let n = remaining.min(chunk.len());
                reader.read_exact(&mut chunk[..n]).await?;
                chunk.zeroize();
                remaining -= n;
            }
            return Ok(false);
        }
        anyhow::ensure!(kind == 0, "Unsupported VNC server message");
        decoder.begin_update();
        reader.read_u8().await?;
        let count = reader.read_u16().await?;
        anyhow::ensure!(count <= MAX_RECTANGLES, "VNC rectangle count exceeds limit");
        let mut budget = 0usize;
        let mut resize_seen = false;
        let mut pixels_seen = false;
        for _ in 0..count {
            let x = reader.read_u16().await?;
            let y = reader.read_u16().await?;
            let w = reader.read_u16().await?;
            let h = reader.read_u16().await?;
            let encoding = reader.read_i32().await?;
            if encoding == -308 {
                anyhow::ensure!(!pixels_seen, "VNC resize update mixes pixels and layout");
                resize_seen = true;
                let screens = reader.read_u8().await?;
                anyhow::ensure!(screens <= 16, "VNC screen count exceeds limit");
                let mut padding = [0; 3];
                reader.read_exact(&mut padding).await?;
                let rejected = x == 1 && y != 0;
                let mut selected = None;
                let mut ids = std::collections::BTreeSet::new();
                for _ in 0..screens {
                    let id = reader.read_u32().await?;
                    let sx = reader.read_u16().await?;
                    let sy = reader.read_u16().await?;
                    let sw = reader.read_u16().await?;
                    let sh = reader.read_u16().await?;
                    let flags = reader.read_u32().await?;
                    if !rejected {
                        anyhow::ensure!(
                            ids.insert(id)
                                && sw > 0
                                && sh > 0
                                && u32::from(sx) + u32::from(sw) <= u32::from(w)
                                && u32::from(sy) + u32::from(sh) <= u32::from(h),
                            "Invalid VNC screen layout"
                        );
                        if screens == 1 && sx == 0 && sy == 0 && sw == w && sh == h {
                            selected = Some(Screen { id, flags });
                        }
                    }
                }
                if rejected {
                    self.resize_status = Some(y);
                    self.resize = None;
                    continue;
                }
                anyhow::ensure!(screens > 0, "Empty VNC screen layout");
                let pixels = w as usize * h as usize;
                anyhow::ensure!(
                    w > 0 && h > 0 && pixels <= MAX_PIXELS,
                    "VNC framebuffer dimensions exceed limit"
                );
                budget = budget
                    .checked_add(pixels * 4)
                    .context("VNC update size overflow")?;
                anyhow::ensure!(budget <= MAX_UPDATE_BYTES, "VNC update work exceeds limit");
                if w != self.width || h != self.height {
                    let cursor = self.cursor.take();
                    *self = Self::new(w, h)?;
                    self.cursor = cursor;
                }
                self.resize = selected;
                self.resize_status = Some(0);
                continue;
            }
            if encoding == -239 {
                anyhow::ensure!(
                    w <= MAX_CURSOR && h <= MAX_CURSOR && (w == 0 || x < w) && (h == 0 || y < h),
                    "Invalid VNC cursor shape"
                );
                let pixels = w as usize * h as usize;
                let stride = (w as usize).div_ceil(8);
                let mut rgba = vec![0; pixels * 4];
                reader.read_exact(&mut rgba).await?;
                let mut mask = vec![0; stride * h as usize];
                reader.read_exact(&mut mask).await?;
                for (i, pixel) in rgba.as_chunks_mut::<4>().0.iter_mut().enumerate() {
                    let (row, column) = (i / w as usize, i % w as usize);
                    let visible = mask[row * stride + column / 8] & (0x80 >> (column % 8)) != 0;
                    *pixel = [pixel[2], pixel[1], pixel[0], if visible { 255 } else { 0 }];
                }
                self.cursor = (w <= SHOWN_CURSOR && h <= SHOWN_CURSOR).then(|| {
                    Arc::new(Cursor {
                        hotspot_x: x,
                        hotspot_y: y,
                        width: w,
                        height: h,
                        rgba,
                    })
                });
                continue;
            }
            anyhow::ensure!(!resize_seen, "VNC resize update mixes pixels and layout");
            pixels_seen = true;
            self.rectangle(x, y, w, h)?;
            let bytes = w as usize * h as usize * 4;
            budget = budget
                .checked_add(bytes)
                .context("VNC update size overflow")?;
            anyhow::ensure!(budget <= MAX_UPDATE_BYTES, "VNC update work exceeds limit");
            match encoding {
                7 => decoder.tight(reader, self, [x, y, w, h]).await?,
                16 => decoder.zrle(reader, self, [x, y, w, h]).await?,
                0 => {
                    // One bounded scanline, never a rectangle-sized wire allocation.
                    let mut row = vec![0; w as usize * 4];
                    for dy in 0..h as usize {
                        reader.read_exact(&mut row).await?;
                        let start = ((y as usize + dy) * self.width as usize + x as usize) * 4;
                        for (out, pixel) in self.rgba[start..start + row.len()]
                            .as_chunks_mut::<4>()
                            .0
                            .iter_mut()
                            .zip(row.as_chunks::<4>().0.iter())
                        {
                            out.copy_from_slice(&[pixel[2], pixel[1], pixel[0], 255]);
                        }
                    }
                }
                1 => {
                    let sx = reader.read_u16().await?;
                    let sy = reader.read_u16().await?;
                    self.rectangle(sx, sy, w, h)?;
                    // memmove semantics in both axes without another framebuffer.
                    for row in 0..h as usize {
                        let dy = if y > sy { h as usize - 1 - row } else { row };
                        let from = ((sy as usize + dy) * self.width as usize + sx as usize) * 4;
                        let to = ((y as usize + dy) * self.width as usize + x as usize) * 4;
                        self.rgba.copy_within(from..from + w as usize * 4, to);
                    }
                }
                _ => bail!("Unsupported VNC framebuffer encoding"),
            }
        }
        Ok(true)
    }
}
