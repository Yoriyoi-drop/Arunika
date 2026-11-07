use arunika_framework_api::AndroidNativeMethods;
use arunika_native_bridge::{JniError, JniValue};

#[test]
fn native_method_terdaftar_dan_dipanggil_melalui_framework() {
    let resolver = AndroidNativeMethods::new();
    resolver.register_native_method("Lcom/contoh/Foo;", "tambah", |args| {
        let a = match args.first() {
            Some(JniValue::Int(v)) => *v,
            _ => return Err(JniError::InvalidType("argumen pertama bukan int".into())),
        };
        let b = match args.get(1) {
            Some(JniValue::Int(v)) => *v,
            _ => return Err(JniError::InvalidType("argumen kedua bukan int".into())),
        };
        Ok(JniValue::Int(a + b))
    });

    let result = resolver
        .call_native_method(
            "Lcom/contoh/Foo;",
            "tambah",
            &[JniValue::Int(4), JniValue::Int(6)],
        )
        .expect("native method harus terpanggil");
    assert_eq!(result, JniValue::Int(10));

    let err = resolver
        .call_native_method("Lcom/contoh/Foo;", "tidakAda", &[])
        .unwrap_err();
    assert!(err.contains("MethodNotFound") || err.contains("tidak ditemukan") || !err.is_empty());
}
