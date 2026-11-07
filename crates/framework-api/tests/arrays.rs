use arunika_dex_engine::{Heap, NativeResolver, Object, Value};
use arunika_framework_api::AndroidNativeMethods;

#[test]
fn arrays_equals_tostring_fill_copyof_sort() {
    let resolver = AndroidNativeMethods::new();
    let mut heap = Heap::new();

    let s_b1 = heap.alloc(Object::String("b".to_string()));
    let s_a1 = heap.alloc(Object::String("a".to_string()));
    let a = heap.alloc(Object::Array {
        element_type: "Ljava/lang/String;".to_string(),
        elements: vec![s_b1, s_a1],
    });
    let s_b2 = heap.alloc(Object::String("b".to_string()));
    let s_a2 = heap.alloc(Object::String("a".to_string()));
    let b = heap.alloc(Object::Array {
        element_type: "Ljava/lang/String;".to_string(),
        elements: vec![s_b2, s_a2],
    });

    let eq = resolver
        .invoke_described(
            &mut heap,
            "Ljava/util/Arrays;",
            "equals",
            "([Ljava/lang/Object;[Ljava/lang/Object;)Z",
            &[a, b],
        )
        .expect("equals tersedia")
        .expect("equals berhasil");
    assert_eq!(eq, Value::Int(1));

    resolver
        .invoke_described(
            &mut heap,
            "Ljava/util/Arrays;",
            "sort",
            "([Ljava/lang/Object;)V",
            &[a],
        )
        .expect("sort tersedia")
        .expect("sort berhasil");

    let text = resolver
        .invoke_described(
            &mut heap,
            "Ljava/util/Arrays;",
            "toString",
            "([Ljava/lang/Object;)Ljava/lang/String;",
            &[a],
        )
        .expect("toString tersedia")
        .expect("toString berhasil");
    let reference = text.as_ref().expect("string");
    assert_eq!(heap.string(reference), Some("[a, b]"));

    let x = heap.alloc(Object::String("x".to_string()));
    resolver
        .invoke_described(
            &mut heap,
            "Ljava/util/Arrays;",
            "fill",
            "([Ljava/lang/Object;Ljava/lang/Object;)V",
            &[a, x],
        )
        .expect("fill tersedia")
        .expect("fill berhasil");

    let copied = resolver
        .invoke_described(
            &mut heap,
            "Ljava/util/Arrays;",
            "copyOf",
            "([Ljava/lang/Object;I)[Ljava/lang/Object;",
            &[a, Value::Int(1)],
        )
        .expect("copyOf tersedia")
        .expect("copyOf berhasil");
    if let Some(reference) = copied.as_ref() {
        if let Some(Object::Array { elements, .. }) = heap.get(reference) {
            assert_eq!(elements.len(), 1);
        } else {
            panic!("hasil copyOf bukan array");
        }
    } else {
        panic!("copyOf null");
    }
}
