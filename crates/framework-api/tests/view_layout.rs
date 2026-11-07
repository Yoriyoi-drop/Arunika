use std::collections::HashMap;

use arunika_dex_engine::{Heap, NativeResolver, Object, Value};
use arunika_framework_api::AndroidNativeMethods;

fn view_instance(heap: &mut Heap, class_name: &str) -> Value {
    heap.alloc(Object::Instance {
        type_name: class_name.to_string(),
        fields: HashMap::new(),
    })
}

#[test]
fn setcontentview_mendaftarkan_view_dan_menata_layout() {
    let resolver = AndroidNativeMethods::new();
    let mut heap = Heap::new();
    let activity = Value::Int(0);
    let view = view_instance(&mut heap, "Landroid/view/View;");

    resolver
        .invoke_described(
            &mut heap,
            "Landroid/app/Activity;",
            "setContentView",
            "(Landroid/view/View;)V",
            &[activity, view],
        )
        .expect("setContentView tersedia")
        .expect("setContentView berhasil");

    let hierarchy = resolver.view_hierarchy();
    assert!(hierarchy.root().is_some(), "root view harus terdaftar");
    let bounds = hierarchy.layout(640, 480);
    assert!(!bounds.is_empty(), "layout harus menghasilkan bounds");
    assert_eq!(resolver.window_state().view_count, hierarchy.len());
    assert!(
        resolver.window_state().draw_calls > 0,
        "layout harus dirender ke offscreen"
    );

    let bitmaps = resolver.graphics_bitmaps();
    assert!(!bitmaps.is_empty(), "bitmap offscreen harus dibuat");
    assert!(
        bitmaps[0].piksel.iter().any(|&byte| byte != 0),
        "render layout harus mengisi piksel"
    );

    // getId pada instance yang sama mengembalikan id node hierarki.
    let id = resolver
        .invoke_described(&mut heap, "Landroid/view/View;", "getId", "()I", &[view])
        .expect("getId tersedia")
        .expect("getId berhasil");
    assert_ne!(id, Value::Int(0));
}

#[test]
fn viewgroup_addview_menambah_child_dan_getchildat() {
    let resolver = AndroidNativeMethods::new();
    let mut heap = Heap::new();
    let parent = view_instance(&mut heap, "Landroid/view/ViewGroup;");
    let child = view_instance(&mut heap, "Landroid/widget/TextView;");

    resolver
        .invoke_described(
            &mut heap,
            "Landroid/view/ViewGroup;",
            "addView",
            "(Landroid/view/View;)V",
            &[parent, child],
        )
        .expect("addView tersedia")
        .expect("addView berhasil");

    let count = resolver
        .invoke_described(
            &mut heap,
            "Landroid/view/ViewGroup;",
            "getChildCount",
            "()I",
            &[parent],
        )
        .expect("getChildCount tersedia")
        .expect("getChildCount berhasil");
    assert_eq!(count, Value::Int(1));

    let at = resolver
        .invoke_described(
            &mut heap,
            "Landroid/view/ViewGroup;",
            "getChildAt",
            "(I)Landroid/view/View;",
            &[parent, Value::Int(0)],
        )
        .expect("getChildAt tersedia")
        .expect("getChildAt berhasil");
    // getChildAt mengembalikan View instance baru yang dipetakan ke child.
    let child_id = resolver
        .invoke_described(&mut heap, "Landroid/view/View;", "getId", "()I", &[at])
        .expect("getId tersedia")
        .expect("getId berhasil");
    assert_ne!(child_id, Value::Int(0));
}
