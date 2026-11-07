//! Mesin grafis untuk fase 3 MVP.
//!
//! Crate ini menyediakan model domain grafis Android (Canvas, Bitmap, Paint,
//! warna) dan operasi rendering dasar ke framebuffer memori. Ia adalah crate
//! domain — tidak bergantung pada host platform dan tidak melakukan render
//! ke layar nyata. Integrasi grafis host (Wayland/X11, GLES/Vulkan, Metal)
//! dilakukan melalui `host-linux`/`host-macos` dan masih belum tersedia.
//!
//! Semua operasi gambar merekam ke `Bitmap` di memori, mirip jendela
//! offscreen fase 1.

use serde::Serialize;
use thiserror::Error;

/// Kesalahan mesin grafis.
#[derive(Debug, Error)]
pub enum GraphicsError {
    #[error("bitmap tidak ditemukan: {0}")]
    BitmapNotFound(u64),
    #[error("ukuran bitmap tidak valid: {lebar}x{tinggi}")]
    InvalidSize { lebar: u32, tinggi: u32 },
    #[error("persegi panjang di luar batas bitmap: {kiri},{atas} {kanan}x{bawah}")]
    RectOutOfBounds {
        kiri: i32,
        atas: i32,
        kanan: i32,
        bawah: i32,
    },
    #[error("warna tidak valid: {0}")]
    InvalidColor(String),
}

/// Warna RGBA 32-bit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Color {
    pub const TRANSPARENT: Color = Color {
        r: 0,
        g: 0,
        b: 0,
        a: 0,
    };
    pub const BLACK: Color = Color {
        r: 0,
        g: 0,
        b: 0,
        a: 255,
    };
    pub const WHITE: Color = Color {
        r: 255,
        g: 255,
        b: 255,
        a: 255,
    };
    pub const RED: Color = Color {
        r: 255,
        g: 0,
        b: 0,
        a: 255,
    };
    pub const GREEN: Color = Color {
        r: 0,
        g: 255,
        b: 0,
        a: 255,
    };
    pub const BLUE: Color = Color {
        r: 0,
        g: 0,
        b: 255,
        a: 255,
    };

    pub fn from_rgba(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }

    pub fn from_argb(argb: u32) -> Self {
        Self {
            a: (argb >> 24) as u8,
            r: (argb >> 16) as u8,
            g: (argb >> 8) as u8,
            b: argb as u8,
        }
    }

    pub fn to_argb(self) -> u32 {
        (self.a as u32) << 24 | (self.r as u32) << 16 | (self.g as u32) << 8 | self.b as u32
    }
}

/// Gaya cat (paint).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum PaintStyle {
    Fill,
    Stroke,
}

/// Cat untuk operasi gambar.
#[derive(Debug, Clone, Serialize)]
pub struct Paint {
    pub warna: Color,
    pub gaya: PaintStyle,
    pub lebar_garis: f32,
    pub anti_alias: bool,
}

impl Default for Paint {
    fn default() -> Self {
        Self {
            warna: Color::BLACK,
            gaya: PaintStyle::Fill,
            lebar_garis: 1.0,
            anti_alias: false,
        }
    }
}

impl Paint {
    pub fn baru() -> Self {
        Self::default()
    }

    pub fn dengan_warna(mut self, warna: Color) -> Self {
        self.warna = warna;
        self
    }

    pub fn dengan_gaya(mut self, gaya: PaintStyle) -> Self {
        self.gaya = gaya;
        self
    }
}

/// Bitmap RGBA di memori.
#[derive(Debug, Clone, Serialize)]
pub struct Bitmap {
    pub id: u64,
    pub lebar: u32,
    pub tinggi: u32,
    /// Piksel RGBA, panjang = lebar * tinggi * 4.
    pub piksel: Vec<u8>,
}

impl Bitmap {
    pub fn baru(id: u64, lebar: u32, tinggi: u32) -> Result<Self, GraphicsError> {
        if lebar == 0 || tinggi == 0 {
            return Err(GraphicsError::InvalidSize { lebar, tinggi });
        }
        let panjang = lebar
            .checked_mul(tinggi)
            .and_then(|p| p.checked_mul(4))
            .ok_or(GraphicsError::InvalidSize { lebar, tinggi })?;
        Ok(Self {
            id,
            lebar,
            tinggi,
            piksel: vec![0; panjang as usize],
        })
    }

    pub fn indeks_piksel(&self, x: u32, y: u32) -> Option<usize> {
        if x >= self.lebar || y >= self.tinggi {
            return None;
        }
        Some((y as usize * self.lebar as usize + x as usize) * 4)
    }

    pub fn atur_piksel(&mut self, x: u32, y: u32, warna: Color) -> Result<(), GraphicsError> {
        let i = self
            .indeks_piksel(x, y)
            .ok_or(GraphicsError::RectOutOfBounds {
                kiri: x as i32,
                atas: y as i32,
                kanan: x as i32 + 1,
                bawah: y as i32 + 1,
            })?;
        self.piksel[i] = warna.r;
        self.piksel[i + 1] = warna.g;
        self.piksel[i + 2] = warna.b;
        self.piksel[i + 3] = warna.a;
        Ok(())
    }

    pub fn baca_piksel(&self, x: u32, y: u32) -> Option<Color> {
        let i = self.indeks_piksel(x, y)?;
        Some(Color {
            r: self.piksel[i],
            g: self.piksel[i + 1],
            b: self.piksel[i + 2],
            a: self.piksel[i + 3],
        })
    }
}

/// Kanvas gambar yang menargetkan sebuah Bitmap.
#[derive(Debug, Clone, Serialize)]
pub struct Canvas {
    pub bitmap_id: u64,
}

impl Canvas {
    pub fn baru(bitmap_id: u64) -> Self {
        Self { bitmap_id }
    }
}

/// Mesin grafis yang mengelola kumpulan Bitmap dan Canvas.
#[derive(Debug, Clone, Serialize, Default)]
pub struct GraphicsEngine {
    bitmaps: std::collections::HashMap<u64, Bitmap>,
    next_id: u64,
}

impl GraphicsEngine {
    pub fn baru() -> Self {
        Self::default()
    }

    /// Buat Bitmap baru dan kembalikan ID-nya.
    pub fn buat_bitmap(&mut self, lebar: u32, tinggi: u32) -> Result<u64, GraphicsError> {
        let id = self.next_id;
        self.next_id += 1;
        let bitmap = Bitmap::baru(id, lebar, tinggi)?;
        self.bitmaps.insert(id, bitmap);
        Ok(id)
    }

    /// Ambil referensi Bitmap berdasarkan ID.
    pub fn bitmap(&self, id: u64) -> Option<&Bitmap> {
        self.bitmaps.get(&id)
    }

    /// Ambil referensi mutable Bitmap berdasarkan ID.
    pub fn bitmap_mut(&mut self, id: u64) -> Option<&mut Bitmap> {
        self.bitmaps.get_mut(&id)
    }

    /// Isi seluruh Bitmap dengan satu warna.
    pub fn draw_color(&mut self, bitmap_id: u64, warna: Color) -> Result<(), GraphicsError> {
        let bitmap = self
            .bitmap_mut(bitmap_id)
            .ok_or(GraphicsError::BitmapNotFound(bitmap_id))?;
        let rgba = [warna.r, warna.g, warna.b, warna.a];
        for chunk in bitmap.piksel.chunks_mut(4) {
            chunk.copy_from_slice(&rgba);
        }
        Ok(())
    }

    /// Gambar persegi panjang terisi.
    pub fn draw_rect(
        &mut self,
        bitmap_id: u64,
        kiri: i32,
        atas: i32,
        kanan: i32,
        bawah: i32,
        warna: Color,
    ) -> Result<(), GraphicsError> {
        let bitmap = self
            .bitmap_mut(bitmap_id)
            .ok_or(GraphicsError::BitmapNotFound(bitmap_id))?;
        let lebar = bitmap.lebar as i32;
        let tinggi = bitmap.tinggi as i32;
        if kiri < 0 || atas < 0 || kanan > lebar || bawah > tinggi || kiri >= kanan || atas >= bawah
        {
            return Err(GraphicsError::RectOutOfBounds {
                kiri,
                atas,
                kanan,
                bawah,
            });
        }
        for y in atas..bawah {
            for x in kiri..kanan {
                let i = (y as usize * lebar as usize + x as usize) * 4;
                bitmap.piksel[i] = warna.r;
                bitmap.piksel[i + 1] = warna.g;
                bitmap.piksel[i + 2] = warna.b;
                bitmap.piksel[i + 3] = warna.a;
            }
        }
        Ok(())
    }

    /// Jumlah bitmap yang dikelola.
    pub fn len(&self) -> usize {
        self.bitmaps.len()
    }

    /// Semua bitmap yang dikelola, diklon untuk pelaporan.
    pub fn bitmaps(&self) -> Vec<Bitmap> {
        self.bitmaps.values().cloned().collect()
    }

    pub fn is_empty(&self) -> bool {
        self.bitmaps.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn engine_awal_kosong() {
        let g = GraphicsEngine::baru();
        assert!(g.is_empty());
        assert_eq!(g.len(), 0);
    }

    #[test]
    fn buat_bitmap_memperoleh_id_dan_piksel_nol() {
        let mut g = GraphicsEngine::baru();
        let id = g.buat_bitmap(2, 2).unwrap();
        assert_eq!(id, 0);
        assert_eq!(g.len(), 1);
        let b = g.bitmap(id).unwrap();
        assert_eq!(b.lebar, 2);
        assert_eq!(b.tinggi, 2);
        assert_eq!(b.piksel.len(), 16);
        assert!(b.piksel.iter().all(|&b| b == 0));
    }

    #[test]
    fn ukuran_bitmap_nol_ditolak() {
        let mut g = GraphicsEngine::baru();
        assert!(g.buat_bitmap(0, 10).is_err());
        assert!(g.buat_bitmap(10, 0).is_err());
        assert!(g.buat_bitmap(0, 0).is_err());
    }

    #[test]
    fn draw_color_isi_semua_piksel() {
        let mut g = GraphicsEngine::baru();
        let id = g.buat_bitmap(2, 2).unwrap();
        g.draw_color(id, Color::RED).unwrap();
        let b = g.bitmap(id).unwrap();
        for y in 0..2 {
            for x in 0..2 {
                assert_eq!(b.baca_piksel(x, y), Some(Color::RED));
            }
        }
    }

    #[test]
    fn draw_rect_terisi() {
        let mut g = GraphicsEngine::baru();
        let id = g.buat_bitmap(4, 4).unwrap();
        g.draw_color(id, Color::WHITE).unwrap();
        g.draw_rect(id, 1, 1, 3, 3, Color::BLUE).unwrap();
        let b = g.bitmap(id).unwrap();
        // Tepi putih, isi biru.
        assert_eq!(b.baca_piksel(0, 0), Some(Color::WHITE));
        assert_eq!(b.baca_piksel(1, 1), Some(Color::BLUE));
        assert_eq!(b.baca_piksel(2, 2), Some(Color::BLUE));
        assert_eq!(b.baca_piksel(3, 3), Some(Color::WHITE));
    }

    #[test]
    fn draw_rect_luar_batas_ditolak() {
        let mut g = GraphicsEngine::baru();
        let id = g.buat_bitmap(4, 4).unwrap();
        let err = g.draw_rect(id, -1, 0, 4, 4, Color::RED).unwrap_err();
        assert!(matches!(err, GraphicsError::RectOutOfBounds { .. }));
    }

    #[test]
    fn bitmap_tidak_ditemukan() {
        let mut g = GraphicsEngine::baru();
        assert!(g.bitmap(99).is_none());
        assert!(g.draw_color(99, Color::RED).is_err());
    }

    #[test]
    fn color_to_argb_dari_dan_ke() {
        let c = Color::from_argb(0xFF112233);
        assert_eq!(c.a, 0xFF);
        assert_eq!(c.r, 0x11);
        assert_eq!(c.g, 0x22);
        assert_eq!(c.b, 0x33);
        assert_eq!(c.to_argb(), 0xFF112233);
    }

    #[test]
    fn paint_default_dan_dengan_warna() {
        let p = Paint::baru()
            .dengan_warna(Color::GREEN)
            .dengan_gaya(PaintStyle::Stroke);
        assert_eq!(p.warna, Color::GREEN);
        assert_eq!(p.gaya, PaintStyle::Stroke);
        assert_eq!(p.lebar_garis, 1.0);
    }

    #[test]
    fn bitmap_serialisasi_json() {
        let b = Bitmap::baru(1, 2, 2).unwrap();
        let json = serde_json::to_string(&b).unwrap();
        assert!(json.contains("\"id\":1"));
        assert!(json.contains("\"lebar\":2"));
    }

    #[test]
    fn graphics_error_tampilkan() {
        let e = GraphicsError::BitmapNotFound(7);
        assert_eq!(e.to_string(), "bitmap tidak ditemukan: 7");
        let e2 = GraphicsError::InvalidSize {
            lebar: 0,
            tinggi: 5,
        };
        assert_eq!(e2.to_string(), "ukuran bitmap tidak valid: 0x5");
        let e3 = GraphicsError::RectOutOfBounds {
            kiri: -1,
            atas: 0,
            kanan: 4,
            bawah: 4,
        };
        assert_eq!(
            e3.to_string(),
            "persegi panjang di luar batas bitmap: -1,0 4x4"
        );
    }
}
