//! Manajemen resource Android untuk fase 2 MVP.
//!
//! Crate ini memiliki satu tanggung jawab: representasi dan resolusi resource
//! aplikasi Android. Ia bukan crate platform — tidak membaca arsip APK secara
//! langsung dan tidak bergantung pada `host-linux`/`host-macos`. Pemanggil
//! (CLI atau lapisan platform) yang bertanggung jawab membaca byte mentah dari
//! entri `res/` APK lalu menyerapkannya ke dalam tabel resource ini.
//!
//! Pada fase 2 MVP, tabel resource APK (`resources.arsc`) belum diurai. ID
//! resource disintesis dari posisi entri per APK (indeks entri + 1), sesuai
//! DESAIN.md. Setiap entri mencatat nama relatif, tipe (`def_type`), dan isi
//! byte mentah.

use serde::Serialize;
use thiserror::Error;

/// Kesalahan manajemen resource.
#[derive(Debug, Error)]
pub enum ResourceError {
    #[error("resource tidak ditemukan: {0}")]
    NotFound(String),
    #[error("id resource tidak valid: {0}")]
    InvalidId(u32),
    #[error("entri resource duplikat: {0}")]
    DuplicateEntry(String),
    #[error("nama resource tidak valid: {0}")]
    InvalidName(String),
}

/// ID resource sintetis yang dihasilkan per APK.
///
/// ID 0 dipersenjatai sebagai tidak ada (mirip `0x0` di Android). ID nyata
/// dimulai dari 1 dan berurutan menurut urutan entri dimasukkan.
pub type ResourceId = u32;

const RESOURCE_ID_NONE: ResourceId = 0;

/// Satu entri resource Android.
#[derive(Debug, Clone, Serialize)]
pub struct ResourceEntry {
    /// ID sintetis entri ini.
    pub id: ResourceId,
    /// Nama relatif entri, misalnya `layout/activity_main.xml` atau
    /// `drawable-mdpi/ic_launcher.png`.
    pub name: String,
    /// Tipe resource (`def_type`), misalnya `layout`, `drawable`, `values`.
    pub def_type: String,
    /// Isi byte mentah resource.
    pub data: Vec<u8>,
}

/// Tabel resource satu APK.
///
/// Entri disimpan dalam urutan masuk. ID sintetis adalah indeks entri + 1,
/// sehingga setiap APK memiliki lingkaran ID sendiri dan ID tidak terpadu
/// lintas APK (sesuai perilaku Android sebenarnya).
#[derive(Debug, Clone, Serialize, Default)]
pub struct ResourceTable {
    entries: Vec<ResourceEntry>,
    name_to_id: std::collections::HashMap<(String, Option<String>), ResourceId>,
}

impl ResourceTable {
    /// Buat tabel resource kosong.
    pub fn new() -> Self {
        Self::default()
    }

    /// Jumlah entri resource dalam tabel.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// `true` bila tabel tidak berisi entri.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Semua entri, terurut menurut ID sintetis.
    pub fn entries(&self) -> &[ResourceEntry] {
        &self.entries
    }

    /// Terapkan nama resource (opsional dengan `def_type`) ke ID sintetis.
    ///
    /// `name` boleh memakai bentuk `folder/nama` (tanpa ekstensi) atau
    /// `folder/nama.ext`. Jika `def_type` diberikan, pencocokan diatur pada
    /// tipe tersebut agar nama yang sama di tipe berbeda tidak saling
    /// bentrok. Jika `def_type` adalah `None`, entri pertama dengan nama
    /// yang cocok dikembalikan, terlepas dari tipe.
    pub fn get_identifier(&self, name: &str, def_type: Option<&str>) -> Option<ResourceId> {
        match def_type {
            Some(dt) => {
                let key = (name.to_owned(), Some(dt.to_owned()));
                self.name_to_id.get(&key).copied()
            }
            None => self
                .entries
                .iter()
                .find(|entry| entry.name == name)
                .map(|entry| entry.id),
        }
    }

    /// Baca isi byte mentah resource berdasarkan ID sintetis.
    pub fn open_raw_resource(&self, id: ResourceId) -> Option<&[u8]> {
        if id == RESOURCE_ID_NONE {
            return None;
        }
        let index = usize::try_from(id).ok()?.checked_sub(1)?;
        self.entries.get(index).map(|e| e.data.as_slice())
    }

    /// Tambahkan satu entri resource.
    ///
    /// Mengembalikan ID sintetis yang ditetapkan. Entri dengan nama + tipe
    /// yang sama ditolak sebagai duplikat.
    pub fn add_entry(
        &mut self,
        name: impl Into<String>,
        def_type: impl Into<String>,
        data: impl Into<Vec<u8>>,
    ) -> Result<ResourceId, ResourceError> {
        let name = name.into();
        let def_type = def_type.into();
        validate_resource_name(&name)?;

        let key = (name.clone(), Some(def_type.clone()));
        if self.name_to_id.contains_key(&key) {
            return Err(ResourceError::DuplicateEntry(format!("{name}::{def_type}")));
        }

        let id = self
            .entries
            .len()
            .checked_add(1)
            .and_then(|n| ResourceId::try_from(n).ok())
            .ok_or(ResourceError::InvalidId(0))?;

        let entry = ResourceEntry {
            id,
            name,
            def_type,
            data: data.into(),
        };
        self.name_to_id.insert(key, id);
        self.entries.push(entry);
        Ok(id)
    }

    /// Ambil entri berdasarkan ID sintetis.
    pub fn entry_by_id(&self, id: ResourceId) -> Option<&ResourceEntry> {
        if id == RESOURCE_ID_NONE {
            return None;
        }
        let index = usize::try_from(id).ok()?.checked_sub(1)?;
        self.entries.get(index)
    }

    /// Hapus semua entri.
    pub fn clear(&mut self) {
        self.entries.clear();
        self.name_to_id.clear();
    }
}

/// Validasi nama resource relatif.
///
/// Nama tidak boleh kosong, tidak boleh berupa path absolut, tidak boleh
/// mengandung `..` atau byte NUL, dan tidak boleh berupa pemisah path saja.
/// Pemisah `/` diperbolehkan karena resource memang bersarang.
fn validate_resource_name(name: &str) -> Result<(), ResourceError> {
    if name.is_empty() {
        return Err(ResourceError::InvalidName(name.to_owned()));
    }
    if name.starts_with('/') || name.contains('\0') {
        return Err(ResourceError::InvalidName(name.to_owned()));
    }
    let components = std::path::Path::new(name).components();
    let mut seen_normal = false;
    for component in components {
        match component {
            std::path::Component::Normal(_) => seen_normal = true,
            std::path::Component::ParentDir
            | std::path::Component::CurDir
            | std::path::Component::RootDir
            | std::path::Component::Prefix(_) => {
                return Err(ResourceError::InvalidName(name.to_owned()));
            }
        }
    }
    if !seen_normal {
        return Err(ResourceError::InvalidName(name.to_owned()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table() -> ResourceTable {
        ResourceTable::new()
    }

    #[test]
    fn tabel_kosong_tidak_memiliki_entri() {
        let t = table();
        assert!(t.is_empty());
        assert_eq!(t.len(), 0);
    }

    #[test]
    fn menambah_entri_menghasilkan_id_berurutan_per_apk() {
        let mut t = table();
        let id1 = t
            .add_entry("layout/activity_main", "layout", b"<xml/>")
            .expect("entri pertama berhasil");
        let id2 = t
            .add_entry("drawable/ic_launcher", "drawable", b"png")
            .expect("entri kedua berhasil");
        assert_eq!(id1, 1);
        assert_eq!(id2, 2);
        assert_eq!(t.len(), 2);
    }

    #[test]
    fn get_identifier_mengembalikan_id_yang_benar() {
        let mut t = table();
        let id = t
            .add_entry("values/strings", "values", b"strings")
            .expect("berhasil");
        assert_eq!(t.get_identifier("values/strings", Some("values")), Some(id));
        assert_eq!(
            t.get_identifier("values/strings", None),
            Some(id),
            "tanpa def_type tetap cocok"
        );
    }

    #[test]
    fn get_identifier_bedakan_tipe_sama_nama() {
        let mut t = table();
        let id_drawable = t
            .add_entry("ic_launcher", "drawable", b"png")
            .expect("drawable");
        let id_layout = t
            .add_entry("ic_launcher", "layout", b"<xml/>")
            .expect("layout");
        assert_ne!(id_drawable, id_layout);
        assert_eq!(
            t.get_identifier("ic_launcher", Some("drawable")),
            Some(id_drawable)
        );
        assert_eq!(
            t.get_identifier("ic_launcher", Some("layout")),
            Some(id_layout)
        );
    }

    #[test]
    fn open_raw_resource_mengembalikan_byte_mentah() {
        let mut t = table();
        let id = t
            .add_entry("raw/hello", "raw", b"halo dunia")
            .expect("berhasil");
        assert_eq!(t.open_raw_resource(id), Some(&b"halo dunia"[..]));
    }

    #[test]
    fn open_raw_resource_id_nol_dan_tidak_ditemukan_mengembalikan_none() {
        let t = table();
        assert_eq!(t.open_raw_resource(0), None);
        assert_eq!(t.open_raw_resource(999), None);
    }

    #[test]
    fn entri_duplikat_ditolak() {
        let mut t = table();
        t.add_entry("values/strings", "values", b"a").unwrap();
        let err = t.add_entry("values/strings", "values", b"b").unwrap_err();
        assert!(matches!(err, ResourceError::DuplicateEntry { .. }));
    }

    #[test]
    fn nama_tidak_valid_ditolak() {
        let mut t = table();
        for bad in ["", "/etc/passwd", "../escape", "a\0b", "."] {
            let err = t.add_entry(bad, "values", b"x").unwrap_err();
            assert!(
                matches!(err, ResourceError::InvalidName { .. }),
                "nama {bad:?} seharusnya ditolak, dapat {err:?}"
            );
        }
    }

    #[test]
    fn entry_by_id_mengembalikan_entri_lengkap() {
        let mut t = table();
        let id = t
            .add_entry("layout/main", "layout", b"<xml/>")
            .expect("berhasil");
        let entry = t.entry_by_id(id).expect("entri ada");
        assert_eq!(entry.id, id);
        assert_eq!(entry.name, "layout/main");
        assert_eq!(entry.def_type, "layout");
        assert_eq!(entry.data, b"<xml/>");
    }

    #[test]
    fn clear_mengosongkan_tabel() {
        let mut t = table();
        t.add_entry("values/strings", "values", b"s").unwrap();
        assert!(!t.is_empty());
        t.clear();
        assert!(t.is_empty());
        assert_eq!(t.get_identifier("values/strings", Some("values")), None);
    }

    #[test]
    fn id_berurutan_lanjut_setelah_hapus_semua() {
        let mut t = table();
        let id1 = t.add_entry("a", "t", b"1").unwrap();
        t.clear();
        let id2 = t.add_entry("b", "t", b"2").unwrap();
        assert_eq!(id1, 1);
        assert_eq!(id2, 1);
    }

    #[test]
    fn resource_entry_dapat_diserialisasi_ke_json() {
        let entry = ResourceEntry {
            id: 1,
            name: "values/strings".to_string(),
            def_type: "values".to_string(),
            data: b"data".to_vec(),
        };
        let json = serde_json::to_string(&entry).expect("dapat diserialisasi");
        assert!(json.contains("values/strings"));
        assert!(json.contains("\"id\":1"));
    }

    #[test]
    fn error_variants_dapat_ditampilkan() {
        let e = ResourceError::NotFound("values/strings".to_string());
        assert_eq!(e.to_string(), "resource tidak ditemukan: values/strings");
        let e2 = ResourceError::InvalidId(999);
        assert_eq!(e2.to_string(), "id resource tidak valid: 999");
        let e3 = ResourceError::DuplicateEntry("x::y".to_string());
        assert_eq!(e3.to_string(), "entri resource duplikat: x::y");
        let e4 = ResourceError::InvalidName("".to_string());
        assert_eq!(e4.to_string(), "nama resource tidak valid: ");
    }
}
