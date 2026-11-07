//! Uji timer kooperatif: tugas yang dijadwalkan lewat java.util.Timer
//! dijalankan oleh engine setelah lifecycle selesai.

use arunika_dex_engine::{DexEngine, Value, parse_bytes};
use arunika_framework_api::AndroidNativeMethods;
use arunika_test_fixtures::dex::{
    ACC_PUBLIC, ACC_STATIC, ClassSpec, FieldSpec, MethodSpec, build_dex_with_symbols, insn,
};

fn kelas_timer() -> ClassSpec {
    ClassSpec::new("Ljava/util/Timer;", Some("Ljava/lang/Object;"))
        .with_method(MethodSpec::new("<init>", ACC_PUBLIC, &[], "V"))
        .with_method(MethodSpec::new(
            "schedule",
            ACC_PUBLIC,
            &["Ljava/util/TimerTask;", "J"],
            "V",
        ))
        .with_method(MethodSpec::new("cancel", ACC_PUBLIC, &[], "V"))
}

fn kelas_timer_task() -> ClassSpec {
    ClassSpec::new("Ljava/util/TimerTask;", Some("Ljava/lang/Object;"))
        .with_method(
            MethodSpec::new("<init>", ACC_PUBLIC, &[], "V")
                .registers(1)
                .ins(1)
                .body(insn::return_void()),
        )
        .with_method(MethodSpec::new("run", ACC_PUBLIC, &[], "V"))
        .with_method(MethodSpec::new("cancel", ACC_PUBLIC, &[], "V"))
}

/// Program uji: memasang satu tugas ke sebuah timer, lalu (opsional) membatalkan
/// timernya. Method run milik tugas menulis 7 ke field statis hit.
#[allow(clippy::too_many_arguments)]
fn build_program(
    timer_type: u16,
    task_type: u16,
    timer_init: u16,
    task_init: u16,
    schedule: u16,
    cancel: Option<u16>,
    hit: u16,
) -> arunika_test_fixtures::dex::DexBuild {
    let task = ClassSpec::new("LMyTask;", Some("Ljava/util/TimerTask;"))
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
                        insn::const_16(0, 7),
                        insn::sput(0, hit),
                        insn::return_void(),
                    ]
                    .concat(),
                ),
        );

    let mut program = [
        insn::new_instance(0, timer_type),
        insn::invoke_direct(timer_init, &[0]),
        insn::new_instance(1, task_type),
        insn::invoke_direct(task_init, &[1]),
        insn::const_wide(2, 0),
        insn::invoke_virtual(schedule, &[0, 1, 2]),
    ]
    .concat();
    if let Some(cancel) = cancel {
        program.extend(insn::invoke_virtual(cancel, &[0]));
    }
    program.extend(insn::return_void());

    let caller = MethodSpec::new("pasang", ACC_PUBLIC | ACC_STATIC, &[], "V")
        .registers(4)
        .body(program);

    let classes = vec![
        kelas_timer(),
        kelas_timer_task(),
        task,
        ClassSpec::new("LContoh;", Some("Ljava/lang/Object;")).with_method(caller),
    ];
    build_dex_with_symbols(&classes)
}

fn engine_dengan_timer(batalkan: bool) -> (DexEngine, u16) {
    let probe = build_program(0, 0, 0, 0, 0, None, 0);
    let timer_type = probe.type_index("Ljava/util/Timer;");
    let task_type = probe.type_index("LMyTask;");
    let timer_init = probe.method_index("Ljava/util/Timer;", "<init>");
    let task_init = probe.method_index("LMyTask;", "<init>");
    let schedule = probe.method_index("Ljava/util/Timer;", "schedule");
    let cancel = batalkan.then(|| probe.method_index("Ljava/util/Timer;", "cancel"));
    let hit = probe.field_index("LMyTask;", "hit");

    let dex_bytes = build_program(
        timer_type, task_type, timer_init, task_init, schedule, cancel, hit,
    )
    .bytes;
    let dex = parse_bytes(&dex_bytes, "timer.dex").expect("DEX fixture harus terparse");
    let mut engine = DexEngine::new(dex);
    engine.set_native_resolver(Box::new(AndroidNativeMethods::new()));
    engine
        .execute_method_values("LContoh;", "pasang", &[])
        .expect("pemasangan timer harus berhasil");
    (engine, hit)
}

#[test]
fn tugas_timer_dijalankan_setelah_lifecycle() {
    let (mut engine, hit) = engine_dengan_timer(false);

    let executed = engine.run_due_timers().expect("tugas timer harus berjalan");
    assert_eq!(executed, 1);
    assert_eq!(engine.statics.get(&(hit as u32)), Some(&Value::Int(7)));
}

#[test]
fn timer_yang_dibatalkan_tidak_menjalankan_tugas() {
    let (mut engine, hit) = engine_dengan_timer(true);

    let executed = engine.run_due_timers().expect("drain timer harus berhasil");
    assert_eq!(executed, 0);
    assert_eq!(engine.statics.get(&(hit as u32)), None);
}
