//! Uji ketahanan loader terhadap APK tidak tepercaya.
//!
//! Loader tidak boleh panic, menggantung, atau mengalokasikan memori tanpa
//! batas hanya karena isi arsip dimanipulasi.

use std::path::{Path, PathBuf};

use arunika_apk_loader::{LoaderError, MAX_DEX_BYTES, dex_read_from_apk, inspect_apk};
use arunika_test_fixtures::{AppSpec, apk_bytes, apk_bytes_with_entries};

fn temp(name: &str, bytes: &[u8]) -> PathBuf {
    let path = std::env::temp_dir().join(format!("arunika-robust-{}-{name}", std::process::id()));
    let _ = std::fs::remove_file(&path);
    std::fs::write(&path, bytes).expect("tidak dapat menulis fixture");
    path
}

fn cleanup(path: &Path) {
    let _ = std::fs::remove_file(path);
}

#[test]
fn pemotongan_apk_di_setiap_offset_tidak_pernah_panic() {
    let base = apk_bytes(&AppSpec::default());

    for len in 0..base.len() {
        let path = temp("potong.apk", &base[..len]);
        let _ = inspect_apk(&path);
        cleanup(&path);
    }
}

#[test]
fn mutasi_byte_apk_tidak_pernah_panic() {
    let base = apk_bytes(&AppSpec::default());

    for offset in 0..base.len() {
        for value in [0x00u8, 0x7f, 0xff] {
            let mut bytes = base.clone();
            bytes[offset] = value;
            let path = temp("mutasi.apk", &bytes);
            let _ = inspect_apk(&path);
            cleanup(&path);
        }
    }
}

#[test]
fn manifest_rusak_ditolak_tanpa_panic() {
    let path = temp(
        "manifest-rusak.apk",
        &arunika_test_fixtures::plain_zip_bytes(&[
            ("AndroidManifest.xml", b"bukan axml"),
            ("classes.dex", &arunika_test_fixtures::minimal_dex()),
        ]),
    );

    assert!(inspect_apk(&path).is_err());
    cleanup(&path);
}

#[test]
fn dex_kosong_terbaca_sebagai_kosong() {
    let path = temp(
        "dex-kosong.apk",
        &apk_bytes_with_entries(&AppSpec::default(), &[("classes.dex", b"")]),
    );

    let report = inspect_apk(&path).expect("APK dengan DEX kosong masih valid secara struktur");
    assert_eq!(report.dex_entries.len(), 1);
    assert_eq!(report.dex_entries[0].size_uncompressed, 0);

    let bytes = dex_read_from_apk(&path, "classes.dex").expect("entry kosong tetap terbaca");
    assert!(bytes.is_empty());

    cleanup(&path);
}

/// Cari offset field uncompressed size pada central directory entry dengan nama tertentu.
fn find_cd_uncompressed_size(apk: &[u8], entry_name: &str) -> Option<usize> {
    let mut offset = 0;
    while let Some(pos) = apk[offset..]
        .windows(4)
        .position(|window| window == b"PK\x01\x02")
    {
        let header = offset + pos;
        let name_len = u16::from_le_bytes([apk[header + 28], apk[header + 29]]) as usize;
        let extra_len = u16::from_le_bytes([apk[header + 30], apk[header + 31]]) as usize;
        let comment_len = u16::from_le_bytes([apk[header + 32], apk[header + 33]]) as usize;
        let name_start = header + 46;
        if apk.len() >= name_start + name_len
            && &apk[name_start..name_start + name_len] == entry_name.as_bytes()
        {
            return Some(header + 24);
        }
        offset = name_start + name_len + extra_len + comment_len;
    }
    None
}

#[test]
fn dex_dengan_ukuran_deklarasi_raksasa_ditolak_sebelum_dibaca() {
    let mut apk = apk_bytes(&AppSpec::default());
    let offset = find_cd_uncompressed_size(&apk, "classes.dex")
        .expect("entry classes.dex harus ditemukan di central directory");

    let declared = (MAX_DEX_BYTES + 1) as u32;
    apk[offset..offset + 4].copy_from_slice(&declared.to_le_bytes());

    let path = temp("dex-besar.apk", &apk);
    let error = dex_read_from_apk(&path, "classes.dex").expect_err("DEX raksasa harus ditolak");
    assert!(
        matches!(error, LoaderError::DexTooLarge { .. }),
        "varian error: {error:?}"
    );

    cleanup(&path);
}
