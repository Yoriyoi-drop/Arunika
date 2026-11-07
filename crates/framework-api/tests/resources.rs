use std::sync::Arc;

use arunika_dex_engine::{Heap, NativeResolver, Object, Value};
use arunika_framework_api::AndroidNativeMethods;
use arunika_sandbox_manager::MemoryAssetSource;

fn teks(heap: &mut Heap, text: &str) -> Value {
    heap.alloc(Object::String(text.to_string()))
}

#[test]
fn resources_get_identifier_dan_open_raw_resource() {
    let mut res = MemoryAssetSource::new();
    res.insert("raw/hello", b"halo".to_vec())
        .expect("resource sah");
    res.insert("drawable/icon", vec![0x89u8, b'P', b'N', b'G'])
        .expect("resource sah");
    let resolver = AndroidNativeMethods::new().with_resources(Arc::new(res));
    let mut heap = Heap::new();
    let nama = teks(&mut heap, "hello");
    let tipe = teks(&mut heap, "raw");
    let paket = teks(&mut heap, "com.x");

    let id = resolver
        .invoke_described(
            &mut heap,
            "Landroid/content/res/Resources;",
            "getIdentifier",
            "(Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;)I",
            &[Value::Int(0), nama, tipe, paket],
        )
        .expect("getIdentifier tersedia")
        .expect("getIdentifier berhasil");
    assert!(
        matches!(id, Value::Int(x) if x != 0),
        "resource harus ditemukan"
    );

    let stream = resolver
        .invoke_described(
            &mut heap,
            "Landroid/content/res/Resources;",
            "openRawResource",
            "(I)Ljava/io/InputStream;",
            &[Value::Int(0), id],
        )
        .expect("openRawResource tersedia")
        .expect("openRawResource berhasil");
    let stream_ref = stream.as_ref().expect("stream");
    match heap.get(stream_ref) {
        Some(Object::Stream { data, .. }) => assert_eq!(data.as_slice(), b"halo"),
        other => panic!("bukan stream: {other:?}"),
    }

    let tname = resolver
        .invoke_described(
            &mut heap,
            "Landroid/content/res/Resources;",
            "getResourceTypeName",
            "(I)Ljava/lang/String;",
            &[Value::Int(0), id],
        )
        .expect("getResourceTypeName tersedia")
        .expect("getResourceTypeName berhasil");
    let tname_ref = tname.as_ref().expect("string");
    assert_eq!(heap.string(tname_ref), Some("raw"));
}

#[test]
fn resources_tanpa_sumber_menghasilkan_id_nol() {
    let resolver = AndroidNativeMethods::new();
    let mut heap = Heap::new();
    let nama = teks(&mut heap, "x");
    let tipe = teks(&mut heap, "raw");
    let paket = teks(&mut heap, "com.x");
    let id = resolver
        .invoke_described(
            &mut heap,
            "Landroid/content/res/Resources;",
            "getIdentifier",
            "(Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;)I",
            &[Value::Int(0), nama, tipe, paket],
        )
        .expect("getIdentifier tersedia")
        .expect("getIdentifier berhasil");
    assert_eq!(id, Value::Int(0), "tanpa sumber, id = 0");
}

#[test]
fn resources_get_identifier_menangani_ekstensi_dan_qualifier() {
    let mut res = MemoryAssetSource::new();
    res.insert("layout/activity_main.xml", b"<xml/>".to_vec())
        .expect("resource sah");
    res.insert("layout-land/activity_main.xml", b"<xml-land/>".to_vec())
        .expect("resource sah");
    let resolver = AndroidNativeMethods::new().with_resources(Arc::new(res));
    let mut heap = Heap::new();
    let nama = teks(&mut heap, "activity_main");
    let tipe = teks(&mut heap, "layout");
    let paket = teks(&mut heap, "com.x");

    let id = resolver
        .invoke_described(
            &mut heap,
            "Landroid/content/res/Resources;",
            "getIdentifier",
            "(Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;)I",
            &[Value::Int(0), nama, tipe, paket],
        )
        .expect("getIdentifier tersedia")
        .expect("getIdentifier berhasil");
    assert!(matches!(id, Value::Int(x) if x != 0));

    let stream = resolver
        .invoke_described(
            &mut heap,
            "Landroid/content/res/Resources;",
            "openRawResource",
            "(I)Ljava/io/InputStream;",
            &[Value::Int(0), id],
        )
        .expect("openRawResource tersedia")
        .expect("openRawResource berhasil");
    let stream_ref = stream.as_ref().expect("stream");
    match heap.get(stream_ref) {
        Some(Object::Stream { data, .. }) => assert_eq!(data.as_slice(), b"<xml/>"),
        other => panic!("bukan stream: {other:?}"),
    }

    let name = resolver
        .invoke_described(
            &mut heap,
            "Landroid/content/res/Resources;",
            "getResourceEntryName",
            "(I)Ljava/lang/String;",
            &[Value::Int(0), id],
        )
        .expect("getResourceEntryName tersedia")
        .expect("getResourceEntryName berhasil");
    let name_ref = name.as_ref().expect("string");
    assert_eq!(heap.string(name_ref), Some("activity_main"));

    let tname = resolver
        .invoke_described(
            &mut heap,
            "Landroid/content/res/Resources;",
            "getResourceTypeName",
            "(I)Ljava/lang/String;",
            &[Value::Int(0), id],
        )
        .expect("getResourceTypeName tersedia")
        .expect("getResourceTypeName berhasil");
    let tname_ref = tname.as_ref().expect("string");
    assert_eq!(heap.string(tname_ref), Some("layout"));
}
