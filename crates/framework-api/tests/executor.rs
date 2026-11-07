//! Uji executor kooperatif: tugas yang diserahkan ke ExecutorService
//! dijalankan oleh engine setelah lifecycle selesai.

use arunika_dex_engine::{DexEngine, Value, parse_bytes};
use arunika_framework_api::AndroidNativeMethods;
use arunika_test_fixtures::dex::{
    ACC_PUBLIC, ACC_STATIC, ClassSpec, FieldSpec, MethodSpec, build_dex_with_symbols, insn,
};

#[test]
fn tugas_executor_dijalankan_setelah_lifecycle() {
    let build = |task_type: u16, factory: u16, execute: u16, task_init: u16, hit: u16| {
        let executors = ClassSpec::new(
            "Ljava/util/concurrent/Executors;",
            Some("Ljava/lang/Object;"),
        )
        .with_method(MethodSpec::new(
            "newFixedThreadPool",
            ACC_PUBLIC | ACC_STATIC,
            &["I"],
            "Ljava/util/concurrent/ExecutorService;",
        ));
        let service = ClassSpec::new(
            "Ljava/util/concurrent/ExecutorService;",
            Some("Ljava/lang/Object;"),
        )
        .with_method(MethodSpec::new(
            "execute",
            ACC_PUBLIC,
            &["Ljava/lang/Runnable;"],
            "V",
        ))
        .with_method(MethodSpec::new("shutdown", ACC_PUBLIC, &[], "V"));
        let task = ClassSpec::new("LMyTask;", Some("Ljava/lang/Object;"))
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
                            insn::const_16(0, 5),
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
                    insn::const_16(0, 2),
                    insn::invoke_static(factory, &[0]),
                    insn::move_result(1),
                    insn::new_instance(2, task_type),
                    insn::invoke_direct(task_init, &[2]),
                    insn::invoke_virtual(execute, &[1, 2]),
                    insn::return_void(),
                ]
                .concat(),
            );

        build_dex_with_symbols(&[
            executors,
            service,
            task,
            ClassSpec::new("LContoh;", Some("Ljava/lang/Object;")).with_method(caller),
        ])
    };

    let probe = build(0, 0, 0, 0, 0);
    let task_type = probe.type_index("LMyTask;");
    let factory = probe.method_index("Ljava/util/concurrent/Executors;", "newFixedThreadPool");
    let execute = probe.method_index("Ljava/util/concurrent/ExecutorService;", "execute");
    let task_init = probe.method_index("LMyTask;", "<init>");
    let hit = probe.field_index("LMyTask;", "hit");

    let dex_bytes = build(task_type, factory, execute, task_init, hit).bytes;
    let dex = parse_bytes(&dex_bytes, "executor.dex").expect("DEX fixture harus terparse");
    let mut engine = DexEngine::new(dex);
    engine.set_native_resolver(Box::new(AndroidNativeMethods::new()));

    engine
        .execute_method_values("LContoh;", "pasang", &[])
        .expect("penyerahan tugas ke executor harus berhasil");

    let executed = engine
        .run_due_executors()
        .expect("tugas executor harus berjalan");
    assert_eq!(executed, 1);
    assert_eq!(engine.statics.get(&(hit as u32)), Some(&Value::Int(5)));
}
