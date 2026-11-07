//! Uji Handler kooperatif: Runnable yang diposting ke main looper dijalankan
//! oleh engine setelah lifecycle selesai.

use arunika_dex_engine::{DexEngine, NativeResolver, Value, parse_bytes};
use arunika_framework_api::AndroidNativeMethods;
use arunika_test_fixtures::dex::{
    ACC_PUBLIC, ACC_STATIC, ClassSpec, FieldSpec, MethodSpec, build_dex_with_symbols, insn,
};

#[test]
fn runnable_yang_diposting_handler_dijalankan_setelah_lifecycle() {
    let build = |handler_type: u16,
                 runnable_type: u16,
                 looper: u16,
                 handler_init: u16,
                 post: u16,
                 runnable_init: u16,
                 hit: u16| {
        let looper_class = ClassSpec::new("Landroid/os/Looper;", Some("Ljava/lang/Object;"))
            .with_method(MethodSpec::new(
                "getMainLooper",
                ACC_PUBLIC | ACC_STATIC,
                &[],
                "Landroid/os/Looper;",
            ));
        let handler = ClassSpec::new("Landroid/os/Handler;", Some("Ljava/lang/Object;"))
            .with_method(MethodSpec::new(
                "<init>",
                ACC_PUBLIC,
                &["Landroid/os/Looper;"],
                "V",
            ))
            .with_method(MethodSpec::new(
                "post",
                ACC_PUBLIC,
                &["Ljava/lang/Runnable;"],
                "Z",
            ));
        let runnable = ClassSpec::new("LMyRunnable;", Some("Ljava/lang/Object;"))
            .with_field(FieldSpec::new("hit", "I", true))
            .with_method(
                MethodSpec::new("<init>", ACC_PUBLIC, &[], "V")
                    .registers(1)
                    .ins(1)
                    .body(insn::return_void()),
            )
            .with_method(
                MethodSpec::new("run", ACC_PUBLIC, &[], "V")
                    .registers(1)
                    .ins(1)
                    .body(
                        [
                            insn::const_16(0, 9),
                            insn::sput(0, hit),
                            insn::return_void(),
                        ]
                        .concat(),
                    ),
            );
        let caller = MethodSpec::new("pasang", ACC_PUBLIC | ACC_STATIC, &[], "V")
            .registers(4)
            .body(
                [
                    insn::invoke_static(looper, &[]),
                    insn::move_result(0),
                    insn::new_instance(1, handler_type),
                    insn::invoke_direct(handler_init, &[1, 0]),
                    insn::new_instance(2, runnable_type),
                    insn::invoke_direct(runnable_init, &[2]),
                    insn::invoke_virtual(post, &[1, 2]),
                    insn::move_result(3),
                    insn::return_void(),
                ]
                .concat(),
            );

        build_dex_with_symbols(&[
            looper_class,
            handler,
            runnable,
            ClassSpec::new("LContoh;", Some("Ljava/lang/Object;")).with_method(caller),
        ])
    };

    let probe = build(0, 0, 0, 0, 0, 0, 0);
    let handler_type = probe.type_index("Landroid/os/Handler;");
    let runnable_type = probe.type_index("LMyRunnable;");
    let looper = probe.method_index("Landroid/os/Looper;", "getMainLooper");
    let handler_init = probe.method_index("Landroid/os/Handler;", "<init>");
    let post = probe.method_index("Landroid/os/Handler;", "post");
    let runnable_init = probe.method_index("LMyRunnable;", "<init>");
    let hit = probe.field_index("LMyRunnable;", "hit");

    let dex_bytes = build(
        handler_type,
        runnable_type,
        looper,
        handler_init,
        post,
        runnable_init,
        hit,
    )
    .bytes;
    let dex = parse_bytes(&dex_bytes, "handler.dex").expect("DEX fixture harus terparse");
    let mut engine = DexEngine::new(dex);
    engine.set_native_resolver(Box::new(AndroidNativeMethods::new()));

    engine
        .execute_method_values("LContoh;", "pasang", &[])
        .expect("posting Runnable harus berhasil");

    let executed = engine
        .run_due_handlers()
        .expect("Runnable yang diposting harus berjalan");
    assert_eq!(executed, 1);
    assert_eq!(engine.statics.get(&(hit as u32)), Some(&Value::Int(9)));
}

#[test]
fn looper_loop_melaporkan_loop_utama_idle() {
    // Runtime satu thread tidak dapat memblokir; mengembalikan segera akan
    // membuat aplikasi berputar pada while(true) { Looper.loop(); }.
    let mut heap = arunika_dex_engine::Heap::new();
    let resolver = AndroidNativeMethods::new();
    let result = resolver
        .invoke_described(&mut heap, "Landroid/os/Looper;", "loop", "()V", &[])
        .expect("loop tersedia")
        .expect_err("loop tanpa tugas harus melaporkan idle");
    assert_eq!(result, arunika_dex_engine::MAIN_LOOP_IDLE);
}
