use std::collections::HashMap;

use arunika_dex_engine::{Heap, NativeResolver, Object, Value};
use arunika_framework_api::AndroidNativeMethods;

fn key_instance(heap: &mut Heap) -> Value {
    heap.alloc(Object::Instance {
        type_name: "Landroid/view/KeyEvent;".to_string(),
        fields: HashMap::new(),
    })
}

#[test]
fn key_event_dibuat_dan_getter_membaca_field() {
    let resolver = AndroidNativeMethods::new();
    let mut heap = Heap::new();
    let instance = key_instance(&mut heap);
    let result = resolver
        .invoke_described(
            &mut heap,
            "Landroid/view/KeyEvent;",
            "<init>",
            "(II)V",
            &[instance, Value::Int(0), Value::Int(66)],
        )
        .expect("KeyEvent.<init> tersedia")
        .expect("KeyEvent.<init> berhasil");
    assert_eq!(result, Value::Int(0));
    let get = |heap: &mut Heap, method: &str| {
        resolver
            .invoke_described(heap, "Landroid/view/KeyEvent;", method, "()I", &[instance])
            .expect("getter tersedia")
            .expect("getter berhasil")
    };
    assert_eq!(get(&mut heap, "getAction"), Value::Int(0));
    assert_eq!(get(&mut heap, "getKeyCode"), Value::Int(66));
    assert_eq!(get(&mut heap, "getRepeatCount"), Value::Int(0));
    let is_down = resolver
        .invoke_described(
            &mut heap,
            "Landroid/view/KeyEvent;",
            "isDown",
            "()Z",
            &[instance],
        )
        .expect("isDown tersedia")
        .expect("isDown berhasil");
    assert_eq!(is_down, Value::Int(1));
}

#[test]
fn key_event_konstruktor_panjang_dan_konstanta() {
    let resolver = AndroidNativeMethods::new();
    let mut heap = Heap::new();
    let instance = key_instance(&mut heap);
    let r = resolver
        .invoke_described(
            &mut heap,
            "Landroid/view/KeyEvent;",
            "<init>",
            "(JJIIII)V",
            &[
                instance,
                Value::from_long(10), // downTime (register 1-2)
                Value::Int(0),        // padding untuk wide (register 2)
                Value::from_long(20), // eventTime (register 3-4)
                Value::Int(0),        // padding untuk wide (register 4)
                Value::Int(1),        // action (register 5)
                Value::Int(4),        // keyCode (register 6)
                Value::Int(2),        // metaState (register 7)
                Value::Int(8),        // repeatCount (register 8)
            ],
        )
        .expect("KeyEvent.<init> panjang tersedia")
        .expect("berhasil");
    assert_eq!(r, Value::Int(0));
    let get_i = |heap: &mut Heap, method: &str| {
        resolver
            .invoke_described(heap, "Landroid/view/KeyEvent;", method, "()I", &[instance])
            .expect("getter tersedia")
            .expect("berhasil")
    };
    assert_eq!(get_i(&mut heap, "getAction"), Value::Int(1));
    assert_eq!(get_i(&mut heap, "getKeyCode"), Value::Int(4));
    assert_eq!(get_i(&mut heap, "getRepeatCount"), Value::Int(2));
    assert_eq!(get_i(&mut heap, "getMetaState"), Value::Int(8));
    let down_time = resolver
        .invoke_described(
            &mut heap,
            "Landroid/view/KeyEvent;",
            "getDownTime",
            "()J",
            &[instance],
        )
        .expect("getDownTime tersedia")
        .expect("berhasil");
    assert_eq!(down_time, Value::from_long(10));
    assert_eq!(
        resolver.field_default(&mut heap, "Landroid/view/KeyEvent;", "ACTION_DOWN"),
        Some(Value::Int(0))
    );
    assert_eq!(
        resolver.field_default(&mut heap, "Landroid/view/KeyEvent;", "KEYCODE_ENTER"),
        Some(Value::Int(66))
    );
    assert_eq!(
        resolver.field_default(&mut heap, "Landroid/view/KeyEvent;", "KEYCODE_BACK"),
        Some(Value::Int(4))
    );
}
