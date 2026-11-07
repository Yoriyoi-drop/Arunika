//! Uji integrasi publik untuk `arunika-sandbox-manager` (fase 2).
//!
//! Memverifikasi API publik sandbox dari luar crate: validasi nama berkas,
//! direktori yang dikonfemen, kebijakan izin, dan sumber aset dalam memori.
use arunika_sandbox_manager::{
    AssetSource, ConfinedRoot, MAX_APP_FILE_BYTES, MAX_ASSET_BYTES, MemoryAssetSource,
    PermissionPolicy, SandboxError, SandboxPolicy, validate_file_name,
};

#[test]
fn kebijakan_strict_menolak_semua_jaringan_dan_silang_aplikasi() {
    let p = SandboxPolicy::default_strict();
    assert!(p.isolated_storage);
    assert!(p.network_default_deny);
    assert!(p.cross_app_data_access_denied);
}

#[test]
fn validasi_nama_berkas_terima_hanya_komponen_biasa() {
    assert_eq!(validate_file_name("cache.dat").unwrap(), "cache.dat");
    assert_eq!(validate_file_name("a_b-c.1").unwrap(), "a_b-c.1");

    for bad in [
        "",
        ".",
        "..",
        "../rahasia",
        "sub/berkas",
        "/etc/passwd",
        "a\\b",
        "a\0b",
    ] {
        assert!(validate_file_name(bad).is_err(), "{bad:?} harus ditolak");
    }
}

#[test]
fn validasi_nama_berkas_terlalu_panjang_ditolak() {
    let terlalu_panjang = "a".repeat(256);
    assert!(validate_file_name(&terlalu_panjang).is_err());
}

#[test]
fn confined_root_resolve_selalu_di_bawah_root() {
    let root = ConfinedRoot::new("/tmp/arunika-uji");
    assert_eq!(
        root.resolve("data.bin").unwrap(),
        std::path::Path::new("/tmp/arunika-uji/data.bin")
    );
    assert!(root.resolve("../luar.bin").is_err());
    assert!(root.resolve("sub/berkas").is_err());
}

#[test]
fn permission_policy_dari_manifest() {
    let mut policy =
        PermissionPolicy::from_permissions(&["android.permission.INTERNET".to_string()]);
    assert!(policy.check("android.permission.INTERNET").is_ok());
    assert!(policy.check("android.permission.CAMERA").is_err());
    policy.grant("android.permission.CAMERA");
    assert!(policy.check("android.permission.CAMERA").is_ok());
    let mut granted = policy.granted();
    granted.sort();
    assert_eq!(
        granted,
        vec![
            "android.permission.CAMERA".to_string(),
            "android.permission.INTERNET".to_string()
        ]
    );
}

#[test]
fn memory_asset_source_read_list_entries() {
    let mut assets = MemoryAssetSource::new();
    assets.insert("boot.bin", b"halo".to_vec()).unwrap();
    assets.insert("config/a.json", b"{}".to_vec()).unwrap();
    assets.insert("config/b.json", b"{}".to_vec()).unwrap();

    assert_eq!(
        assets.read("boot.bin").unwrap().as_deref(),
        Some(&b"halo"[..])
    );
    assert!(assets.read("tidak-ada.bin").unwrap().is_none());
    assert!(assets.read("../luar.bin").is_err());

    assert_eq!(
        assets.list("").unwrap(),
        vec!["boot.bin".to_string(), "config".to_string()]
    );
    assert_eq!(
        assets.list("config").unwrap(),
        vec!["a.json".to_string(), "b.json".to_string()]
    );
    assert_eq!(
        assets.entries().unwrap(),
        vec![
            "boot.bin".to_string(),
            "config/a.json".to_string(),
            "config/b.json".to_string()
        ]
    );
}

#[test]
fn asset_source_membatasi_ukuran_dan_nama() {
    let mut assets = MemoryAssetSource::new();
    let terlalu_besar = vec![0u8; MAX_ASSET_BYTES + 1];
    assert!(assets.insert("besar.bin", terlalu_besar).is_err());

    let nama_salah = ["", "/etc/passwd", "../luar", "a\\b", "a\0b"];
    for bad in nama_salah {
        assert!(assets.insert(bad, b"x").is_err(), "{bad:?} harus ditolak");
    }
}

#[test]
fn asset_children_satu_tingkat() {
    let nama = vec![
        "boot.bin".to_string(),
        "config/a.json".to_string(),
        "config/b.json".to_string(),
        "config/sub/c.json".to_string(),
        "res/ic.png".to_string(),
    ];
    assert_eq!(
        arunika_sandbox_manager::asset_children(nama.clone(), "config"),
        vec![
            "a.json".to_string(),
            "b.json".to_string(),
            "sub".to_string()
        ]
    );
    assert_eq!(
        arunika_sandbox_manager::asset_children(nama, ""),
        vec![
            "boot.bin".to_string(),
            "config".to_string(),
            "res".to_string()
        ]
    );
}

#[test]
fn konstanta_batas_dokumen() {
    assert_eq!(MAX_APP_FILE_BYTES, 8 * 1024 * 1024);
    assert_eq!(MAX_ASSET_BYTES, 16 * 1024 * 1024);
}

#[test]
fn sandbox_error_tampilkan_pesan_jelas() {
    let err = SandboxError::PathEscape("/etc/passwd".to_string());
    assert!(err.to_string().contains("/etc/passwd"));
    let err = SandboxError::Denied("android.permission.CAMERA".to_string());
    assert!(err.to_string().contains("CAMERA"));
    let err = SandboxError::TooLarge {
        name: "big.bin".to_string(),
        limit: 100,
    };
    assert!(err.to_string().contains("big.bin"));
    assert!(err.to_string().contains("100"));
}
