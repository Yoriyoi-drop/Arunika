//! Uji integrasi publik untuk `arunika-resource-manager` (fase 2 MVP).
//!
//! Memverifikasi tabel resource sintetis: penambahan entri, pencarian ID,
//! pembacaan byte mentah, dan penolakan duplikat/nama tidak valid.

use arunika_resource_manager::{ResourceError, ResourceTable};

#[test]
fn tabel_resource_kosong_saat_dibuat() {
    let tabel = ResourceTable::new();
    assert!(tabel.is_empty());
    assert_eq!(tabel.len(), 0);
}

#[test]
fn tambah_entri_mengembalikan_id_berurutan() {
    let mut tabel = ResourceTable::new();
    let id1 = tabel
        .add_entry("layout/main", "layout", b"<xml/>")
        .expect("entri pertama");
    let id2 = tabel
        .add_entry("drawable/icon", "drawable", b"PNG")
        .expect("entri kedua");
    assert_eq!(id1, 1);
    assert_eq!(id2, 2);
    assert_eq!(tabel.len(), 2);
}

#[test]
fn get_identifier_dengan_type_mencocokkan_type_tepat() {
    let mut tabel = ResourceTable::new();
    tabel.add_entry("ic", "drawable", b"a").unwrap();
    tabel.add_entry("ic", "layout", b"<xml/>").unwrap();

    assert_eq!(tabel.get_identifier("ic", Some("drawable")), Some(1));
    assert_eq!(tabel.get_identifier("ic", Some("layout")), Some(2));
    assert_eq!(tabel.get_identifier("ic", None), Some(1));
}

#[test]
fn open_raw_resource_mengembalikan_data_mentah() {
    let mut tabel = ResourceTable::new();
    let id = tabel.add_entry("data", "raw", b"hello world").unwrap();
    assert_eq!(tabel.open_raw_resource(id), Some(&b"hello world"[..]));
    assert_eq!(tabel.open_raw_resource(0), None);
    assert_eq!(tabel.open_raw_resource(999), None);
}

#[test]
fn entri_duplikat_dengan_nama_dan_type_sama_ditolak() {
    let mut tabel = ResourceTable::new();
    tabel.add_entry("img", "drawable", b"a").unwrap();
    let error = tabel.add_entry("img", "drawable", b"b").unwrap_err();
    assert!(matches!(error, ResourceError::DuplicateEntry { .. }));
}

#[test]
fn nama_resource_tidak_valid_ditolak() {
    let mut tabel = ResourceTable::new();
    for bad in ["", "/abs", "..", "a\0b"] {
        assert!(
            tabel.add_entry(bad, "drawable", b"x").is_err(),
            "{bad:?} harus ditolak"
        );
    }
}

#[test]
fn semua_entri_dapat_diakses_berurutan() {
    let mut tabel = ResourceTable::new();
    tabel.add_entry("a", "drawable", b"1").unwrap();
    tabel.add_entry("b", "layout", b"2").unwrap();
    tabel.add_entry("c", "values", b"3").unwrap();

    let entries: Vec<_> = tabel.entries().iter().map(|e| e.id).collect();
    assert_eq!(entries, vec![1, 2, 3]);
}
