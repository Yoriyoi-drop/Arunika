//! Uji `AssetManager`: aset read-only dari APK dibaca sebagai aliran memori.

use std::sync::Arc;

use arunika_dex_engine::{Heap, NativeResolver, Object, Value};
use arunika_framework_api::AndroidNativeMethods;
use arunika_sandbox_manager::MemoryAssetSource;

fn teks(heap: &mut Heap, text: &str) -> Value {
    heap.alloc(Object::String(text.to_string()))
}

fn konteks(heap: &mut Heap) -> Value {
    heap.alloc(Object::Instance {
        type_name: "Landroid/app/Activity;".to_string(),
        fields: std::collections::HashMap::new(),
    })
}

fn resolver_dengan_aset() -> AndroidNativeMethods {
    let mut assets = MemoryAssetSource::new();
    assets
        .insert("config/level1.json", b"{\"level\":1}".to_vec())
        .expect("aset sah");
    assets
        .insert("boot.bin", b"halo".to_vec())
        .expect("aset sah");
    AndroidNativeMethods::new().with_assets(Arc::new(assets))
}

#[test]
fn asset_manager_membuka_aset_sebagai_aliran_dan_menghabisinya() {
    let resolver = resolver_dengan_aset();
    let mut heap = Heap::new();
    let context = konteks(&mut heap);

    let manager = resolver
        .invoke_described(
            &mut heap,
            "Landroid/app/Activity;",
            "getAssets",
            "()Landroid/content/res/AssetManager;",
            &[context],
        )
        .expect("getAssets tersedia")
        .expect("getAssets berhasil");

    let name = teks(&mut heap, "config/level1.json");
    let stream = resolver
        .invoke_described(
            &mut heap,
            "Landroid/content/res/AssetManager;",
            "open",
            "(Ljava/lang/String;)Ljava/io/InputStream;",
            &[manager, name],
        )
        .expect("open tersedia")
        .expect("aset harus ditemukan");

    let buffer = heap.alloc(Object::Array {
        element_type: "B".to_string(),
        elements: vec![Value::Int(0); 32],
    });
    let read = resolver
        .invoke_described(
            &mut heap,
            "Ljava/io/InputStream;",
            "read",
            "([B)I",
            &[stream, buffer],
        )
        .expect("read tersedia")
        .expect("read berhasil");
    assert_eq!(read.as_int(), Some(11), "isi aset harus terbaca utuh");

    let buffer_ref = buffer.as_ref().expect("array");
    match heap.get(buffer_ref) {
        Some(Object::Array { elements, .. }) => {
            assert_eq!(elements[0].as_int(), Some(i32::from(b'{')));
            assert_eq!(elements[10].as_int(), Some(i32::from(b'}')));
        }
        other => panic!("buffer bukan array: {other:?}"),
    }

    let sisa = resolver
        .invoke_described(
            &mut heap,
            "Ljava/io/InputStream;",
            "available",
            "()I",
            &[stream],
        )
        .expect("available tersedia")
        .expect("available berhasil");
    assert_eq!(sisa.as_int(), Some(0), "aliran sudah habis");
}

#[test]
fn asset_manager_melaporkan_aset_hilang_sebagai_file_not_found() {
    let resolver = resolver_dengan_aset();
    let mut heap = Heap::new();
    let context = konteks(&mut heap);
    let manager = resolver
        .invoke_described(
            &mut heap,
            "Landroid/app/Activity;",
            "getAssets",
            "()Landroid/content/res/AssetManager;",
            &[context],
        )
        .expect("getAssets tersedia")
        .expect("getAssets berhasil");

    let name = teks(&mut heap, "tidak-ada.bin");
    let error = resolver
        .invoke_described(
            &mut heap,
            "Landroid/content/res/AssetManager;",
            "open",
            "(Ljava/lang/String;)Ljava/io/InputStream;",
            &[manager, name],
        )
        .expect("open tersedia")
        .expect_err("aset yang tidak ada harus gagal");
    assert!(
        error.contains("java.io.FileNotFoundException"),
        "galat harus berupa FileNotFoundException, bukan {error}"
    );
}

#[test]
fn asset_manager_mendaftar_anak_direktori_dan_menolak_tanpa_sumber() {
    let resolver = resolver_dengan_aset();
    let mut heap = Heap::new();
    let context = konteks(&mut heap);
    let manager = resolver
        .invoke_described(
            &mut heap,
            "Landroid/app/Activity;",
            "getAssets",
            "()Landroid/content/res/AssetManager;",
            &[context],
        )
        .expect("getAssets tersedia")
        .expect("getAssets berhasil");

    let prefix = teks(&mut heap, "config");
    let list = resolver
        .invoke_described(
            &mut heap,
            "Landroid/content/res/AssetManager;",
            "list",
            "(Ljava/lang/String;)[Ljava/lang/String;",
            &[manager, prefix],
        )
        .expect("list tersedia")
        .expect("list berhasil");
    let list_ref = list.as_ref().expect("array");
    match heap.get(list_ref) {
        Some(Object::Array { elements, .. }) => {
            assert_eq!(elements.len(), 1);
            let child = elements[0].as_ref().expect("String");
            assert_eq!(heap.string(child), Some("level1.json"));
        }
        other => panic!("list bukan array: {other:?}"),
    }

    // Tanpa sumber aset, runtime melaporkan keterbatasan alih-alih
    // mengembalikan aliran kosong.
    let tanpa = AndroidNativeMethods::new();
    let name = teks(&mut heap, "boot.bin");
    let error = tanpa
        .invoke_described(
            &mut heap,
            "Landroid/content/res/AssetManager;",
            "open",
            "(Ljava/lang/String;)Ljava/io/InputStream;",
            &[manager, name],
        )
        .expect("open tersedia")
        .expect_err("tanpa aset harus gagal");
    assert!(error.contains("tidak diberi akses aset"));
}
