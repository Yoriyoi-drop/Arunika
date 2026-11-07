use std::collections::HashMap;

use arunika_dex_engine::{Heap, NativeResolver, Object, Value};
use arunika_framework_api::AndroidNativeMethods;

fn canvas_instance(heap: &mut Heap) -> Value {
    heap.alloc(Object::Instance {
        type_name: "Landroid/graphics/Canvas;".to_string(),
        fields: HashMap::new(),
    })
}

#[test]
fn bitmap_create_dan_ukuran() {
    let resolver = AndroidNativeMethods::new();
    let mut heap = Heap::new();
    let bitmap = resolver
        .invoke_described(
            &mut heap,
            "Landroid/graphics/Bitmap;",
            "createBitmap",
            "(IILandroid/graphics/Bitmap$Config;)Landroid/graphics/Bitmap;",
            &[Value::Int(12), Value::Int(34), Value::Int(0)],
        )
        .expect("createBitmap tersedia")
        .expect("createBitmap berhasil");
    let w = resolver
        .invoke_described(
            &mut heap,
            "Landroid/graphics/Bitmap;",
            "getWidth",
            "()I",
            &[bitmap],
        )
        .expect("getWidth tersedia")
        .expect("getWidth berhasil");
    let h = resolver
        .invoke_described(
            &mut heap,
            "Landroid/graphics/Bitmap;",
            "getHeight",
            "()I",
            &[bitmap],
        )
        .expect("getHeight tersedia")
        .expect("getHeight berhasil");
    assert_eq!(w, Value::Int(12));
    assert_eq!(h, Value::Int(34));
}

#[test]
fn kanvas_menggambar_merekam_state_jendela() {
    let resolver = AndroidNativeMethods::new();
    let mut heap = Heap::new();
    let bitmap = resolver
        .invoke_described(
            &mut heap,
            "Landroid/graphics/Bitmap;",
            "createBitmap",
            "(IILandroid/graphics/Bitmap$Config;)Landroid/graphics/Bitmap;",
            &[Value::Int(8), Value::Int(8), Value::Int(0)],
        )
        .expect("createBitmap berhasil")
        .expect("bitmap");
    let canvas = canvas_instance(&mut heap);
    resolver
        .invoke_described(
            &mut heap,
            "Landroid/graphics/Canvas;",
            "<init>",
            "(Landroid/graphics/Bitmap;)V",
            &[canvas, bitmap],
        )
        .expect("init tersedia")
        .expect("init berhasil");
    resolver
        .invoke_described(
            &mut heap,
            "Landroid/graphics/Canvas;",
            "drawColor",
            "(I)V",
            &[canvas, Value::Int(0x11223333)],
        )
        .expect("drawColor tersedia")
        .expect("drawColor berhasil");
    resolver
        .invoke_described(
            &mut heap,
            "Landroid/graphics/Canvas;",
            "drawRect",
            "(FFFFI)V",
            &[
                canvas,
                Value::Int(0),
                Value::Int(0),
                Value::Int(4),
                Value::Int(4),
                Value::Int(0x000000FF),
            ],
        )
        .expect("drawRect tersedia")
        .expect("drawRect berhasil");

    let state = resolver.window_state();
    assert!(
        state.present,
        "jendela harus ditandai terpakai setelah menggambar"
    );
    assert_eq!(state.draw_calls, 2, "dua operasi gambar terekam");
}

#[test]
fn window_setcontentview_menandai_jendela_dengan_dimensi_default() {
    let resolver = AndroidNativeMethods::new();
    let mut heap = Heap::new();
    let window = resolver
        .invoke_described(
            &mut heap,
            "Landroid/app/Activity;",
            "getWindow",
            "()Landroid/view/Window;",
            &[Value::Int(0)],
        )
        .expect("getWindow tersedia")
        .expect("getWindow berhasil");
    resolver
        .invoke_described(
            &mut heap,
            "Landroid/view/Window;",
            "setContentView",
            "(I)V",
            &[window, Value::Int(2131230768)],
        )
        .expect("setContentView tersedia")
        .expect("setContentView berhasil");
    let state = resolver.window_state();
    assert!(state.present);
    assert!(
        state.width > 0 && state.height > 0,
        "dimensi virtual default diisi"
    );
}
