//! Poin layanan aplikasi untuk fase 2 MVP.
//!
//! Crate ini menyediakan model layanan lintas aplikasi yang diatur oleh
//! runtime, dimulai dengan clipboard. Ia adalah crate domain — tidak
//! bergantung pada platform host dan tidak menyediakan implementasi
//! clipboard nyata di luar proses. Pemanggilan ke layanan sistem sesungguhnya
//! diserahkan ke `host-linux`/`host-macos` melalui kontrak host yang eksplisit.
//!
//! Clipboard di sini adalah memori dalam proses: satu slot primer per runtime
//! yang menyimpan `ClipData`. Ini mencakup kebutuhan Fase 2 MVP untuk
//! menyalin/tempel teks antar aplikasi dalam satu proses runtime.

use serde::Serialize;
use std::collections::HashMap;
use thiserror::Error;

/// Kesalahan layanan aplikasi.
#[derive(Debug, Error)]
pub enum ServiceError {
    #[error("layanan tidak tersedia: {0}")]
    Unavailable(String),
    #[error("clipboard kosong: {0}")]
    EmptyClipboard(String),
    #[error("data clipboard tidak valid: {0}")]
    InvalidClipData(String),
}

/// Data clipboard: teks opsional, HTML opsional, dan label.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Default)]
pub struct ClipData {
    /// Teks biasa.
    pub text: Option<String>,
    /// Teks HTML (opsional, untuk aplikasi yang mendukung rich text).
    pub html: Option<String>,
    /// Label deskriptif untuk clipboard.
    pub label: Option<String>,
    /// Ekstra parameter tambahan (mis. MIME type).
    pub extras: HashMap<String, String>,
}

impl ClipData {
    pub fn new() -> Self {
        Self::default()
    }

    /// Buat ClipData hanya teks.
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            text: Some(text.into()),
            ..Self::default()
        }
    }

    /// Apakah data ini tidak mengandung apa pun.
    pub fn is_empty(&self) -> bool {
        self.text.is_none() && self.html.is_none() && self.label.is_none() && self.extras.is_empty()
    }

    /// Tambahkan pasangan kunci/nilai ke extras.
    pub fn with_extra(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.extras.insert(key.into(), value.into());
        self
    }
}

/// Manajer clipboard dalam proses.
///
/// Simpan satu slot primer per instance runtime. Untuk produksi, ini akan
/// diganti dengan integrasi clipboard host melalui `host-linux`/`host-macos`.
#[derive(Debug, Clone, Serialize, Default)]
pub struct ClipboardManager {
    primary: Option<ClipData>,
    /// Riwayat clipboard terakhir (opsional, untuk riwayat tempel).
    history: Vec<ClipData>,
    /// Batas maksimal riwayat agar tidak tumbuh tanpa batas.
    max_history: usize,
}

impl ClipboardManager {
    /// Buat manajer clipboard kosong.
    pub fn new() -> Self {
        Self::default()
    }

    /// Buat manajer dengan batas riwayat tertentu.
    pub fn with_max_history(max_history: usize) -> Self {
        Self {
            max_history,
            ..Self::default()
        }
    }

    /// Apakah slot primer saat ini berisi data.
    pub fn has_primary_clip(&self) -> bool {
        self.primary
            .as_ref()
            .map(|c| !c.is_empty())
            .unwrap_or(false)
    }

    /// Ambil salinan data clipboard primer, atau error bila kosong.
    pub fn get_primary_clip(&self) -> Result<ClipData, ServiceError> {
        self.primary
            .clone()
            .ok_or_else(|| ServiceError::EmptyClipboard("clipboard primer kosong".into()))
    }

    /// Tetapkan data clipboard primer. Data lama dipindahkan ke riwayat.
    pub fn set_primary_clip(&mut self, data: ClipData) -> Result<(), ServiceError> {
        if data.is_empty() {
            return Err(ServiceError::InvalidClipData(
                "tidak boleh menyetorkan clipboard kosong".into(),
            ));
        }
        if let Some(existing) = self.primary.take()
            && !existing.is_empty()
            && self.history.len() < self.max_history
        {
            self.history.push(existing);
        }
        self.primary = Some(data);
        Ok(())
    }

    /// Hapus clipboard primer.
    pub fn clear_primary_clip(&mut self) -> Result<(), ServiceError> {
        if self.primary.is_none() {
            return Err(ServiceError::EmptyClipboard(
                "clipboard sudah kosong".into(),
            ));
        }
        self.primary = None;
        Ok(())
    }

    /// Jumlah item dalam riwayat clipboard.
    pub fn history_len(&self) -> usize {
        self.history.len()
    }

    /// Ambil item riwayat berdasarkan indeks (terbaru = indeks 0).
    pub fn history_at(&self, index: usize) -> Option<&ClipData> {
        self.history.get(index)
    }

    /// Batas riwayat saat ini.
    pub fn max_history(&self) -> usize {
        self.max_history
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clipboard_awal_kosong() {
        let cb = ClipboardManager::new();
        assert!(!cb.has_primary_clip());
        assert_eq!(cb.history_len(), 0);
    }

    #[test]
    fn set_dan_get_primary_clip() {
        let mut cb = ClipboardManager::new();
        let data = ClipData::text("halo dunia".to_string());
        cb.set_primary_clip(data).unwrap();
        assert!(cb.has_primary_clip());

        let got = cb.get_primary_clip().unwrap();
        assert_eq!(got.text.as_deref(), Some("halo dunia"));
    }

    #[test]
    fn get_clipboard_kosong_mengembalikan_error() {
        let cb = ClipboardManager::new();
        let err = cb.get_primary_clip().unwrap_err();
        assert!(matches!(err, ServiceError::EmptyClipboard { .. }));
    }

    #[test]
    fn set_clipboard_kosong_ditolak() {
        let mut cb = ClipboardManager::new();
        let err = cb.set_primary_clip(ClipData::new()).unwrap_err();
        assert!(matches!(err, ServiceError::InvalidClipData { .. }));
    }

    #[test]
    fn clear_primary_clip() {
        let mut cb = ClipboardManager::new();
        cb.set_primary_clip(ClipData::text("data".to_string()))
            .unwrap();
        cb.clear_primary_clip().unwrap();
        assert!(!cb.has_primary_clip());
        let err = cb.clear_primary_clip().unwrap_err();
        assert!(matches!(err, ServiceError::EmptyClipboard { .. }));
    }

    #[test]
    fn data_lama_pindah_ke_riwayat() {
        let mut cb = ClipboardManager::with_max_history(5);
        cb.set_primary_clip(ClipData::text("pertama".to_string()))
            .unwrap();
        cb.set_primary_clip(ClipData::text("kedua".to_string()))
            .unwrap();
        assert_eq!(cb.history_len(), 1);
        assert_eq!(cb.history_at(0).unwrap().text.as_deref(), Some("pertama"));
        assert_eq!(
            cb.get_primary_clip().unwrap().text.as_deref(),
            Some("kedua")
        );
    }

    #[test]
    fn riwayat_dibatasi_max_history() {
        let mut cb = ClipboardManager::with_max_history(2);
        for nama in ["a", "b", "c", "d"] {
            cb.set_primary_clip(ClipData::text(nama.to_string()))
                .unwrap();
        }
        assert_eq!(cb.history_len(), 2);
    }

    #[test]
    fn clipdata_text_dan_default() {
        let t = ClipData::text("teks".to_string());
        assert_eq!(t.text.as_deref(), Some("teks"));
        assert!(!t.is_empty());

        let kosong = ClipData::new();
        assert!(kosong.is_empty());
    }

    #[test]
    fn clipdata_dengan_extra() {
        let data = ClipData::text("isi").with_extra("mime", "text/plain");
        assert_eq!(data.extras.get("mime"), Some(&"text/plain".to_string()));
    }

    #[test]
    fn clipdata_serialisasi_json() {
        let data = ClipData::text("halo");
        let json = serde_json::to_string(&data).unwrap();
        assert!(json.contains("halo"));
    }

    #[test]
    fn service_error_tampilkan() {
        let e = ServiceError::Unavailable("clipboard-host".into());
        assert_eq!(e.to_string(), "layanan tidak tersedia: clipboard-host");
        let e2 = ServiceError::EmptyClipboard("kosong".into());
        assert_eq!(e2.to_string(), "clipboard kosong: kosong");
        let e3 = ServiceError::InvalidClipData("buruk".into());
        assert_eq!(e3.to_string(), "data clipboard tidak valid: buruk");
    }
}
