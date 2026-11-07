use arunika_dex_engine::{Heap, NativeResolver, Object, Value};
use arunika_framework_api::AndroidNativeMethods;

#[test]
fn objects_equals_dan_hashcode() {
    let resolver = AndroidNativeMethods::new();
    let mut heap = Heap::new();
    let a = heap.alloc(Object::String("halo".to_string()));
    let b = heap.alloc(Object::String("halo".to_string()));
    let c = heap.alloc(Object::String("lain".to_string()));

    let eq = resolver
        .invoke_described(
            &mut heap,
            "Ljava/util/Objects;",
            "equals",
            "(Ljava/lang/Object;Ljava/lang/Object;)Z",
            &[a, b],
        )
        .expect("equals tersedia")
        .expect("equals berhasil");
    assert_eq!(eq, Value::Int(1));

    let eq2 = resolver
        .invoke_described(
            &mut heap,
            "Ljava/util/Objects;",
            "equals",
            "(Ljava/lang/Object;Ljava/lang/Object;)Z",
            &[a, c],
        )
        .expect("equals tersedia")
        .expect("equals berhasil");
    assert_eq!(eq2, Value::Int(0));

    let h1 = resolver
        .invoke_described(
            &mut heap,
            "Ljava/util/Objects;",
            "hashCode",
            "(Ljava/lang/Object;)I",
            &[a],
        )
        .expect("hashCode tersedia")
        .expect("hashCode berhasil");
    let h0 = resolver
        .invoke_described(
            &mut heap,
            "Ljava/util/Objects;",
            "hashCode",
            "(Ljava/lang/Object;)I",
            &[Value::Int(0)],
        )
        .expect("hashCode tersedia")
        .expect("hashCode berhasil");
    assert_eq!(h0, Value::Int(0));
    assert_ne!(h1, Value::Int(0));
}

#[test]
fn objects_require_non_null_dan_is_null() {
    let resolver = AndroidNativeMethods::new();
    let mut heap = Heap::new();
    let value = heap.alloc(Object::String("x".to_string()));

    let ok = resolver
        .invoke_described(
            &mut heap,
            "Ljava/util/Objects;",
            "requireNonNull",
            "(Ljava/lang/Object;)Ljava/lang/Object;",
            &[value],
        )
        .expect("requireNonNull tersedia")
        .expect("requireNonNull berhasil");
    assert_eq!(ok, value);

    let is_null = resolver
        .invoke_described(
            &mut heap,
            "Ljava/util/Objects;",
            "isNull",
            "(Ljava/lang/Object;)Z",
            &[Value::Int(0)],
        )
        .expect("isNull tersedia")
        .expect("isNull berhasil");
    assert_eq!(is_null, Value::Int(1));

    let non_null = resolver
        .invoke_described(
            &mut heap,
            "Ljava/util/Objects;",
            "nonNull",
            "(Ljava/lang/Object;)Z",
            &[value],
        )
        .expect("nonNull tersedia")
        .expect("nonNull berhasil");
    assert_eq!(non_null, Value::Int(1));

    let result = resolver.invoke_described(
        &mut heap,
        "Ljava/util/Objects;",
        "requireNonNull",
        "(Ljava/lang/Object;)Ljava/lang/Object;",
        &[Value::Int(0)],
    );
    assert!(result.is_some());
    assert!(result.unwrap().is_err());
}

#[test]
fn objects_to_string_dan_deep_equals() {
    let resolver = AndroidNativeMethods::new();
    let mut heap = Heap::new();
    let a = heap.alloc(Object::String("halo".to_string()));
    let b = heap.alloc(Object::String("halo".to_string()));

    let text = resolver
        .invoke_described(
            &mut heap,
            "Ljava/util/Objects;",
            "toString",
            "(Ljava/lang/Object;)Ljava/lang/String;",
            &[a],
        )
        .expect("toString tersedia")
        .expect("toString berhasil");
    let reference = text.as_ref().expect("string");
    assert_eq!(heap.string(reference), Some("halo"));

    let null_text = resolver
        .invoke_described(
            &mut heap,
            "Ljava/util/Objects;",
            "toString",
            "(Ljava/lang/Object;)Ljava/lang/String;",
            &[Value::Int(0)],
        )
        .expect("toString tersedia")
        .expect("toString berhasil");
    let reference = null_text.as_ref().expect("string");
    assert_eq!(heap.string(reference), Some("null"));

    let deep = resolver
        .invoke_described(
            &mut heap,
            "Ljava/util/Objects;",
            "deepEquals",
            "(Ljava/lang/Object;Ljava/lang/Object;)Z",
            &[a, b],
        )
        .expect("deepEquals tersedia")
        .expect("deepEquals berhasil");
    assert_eq!(deep, Value::Int(1));
}
