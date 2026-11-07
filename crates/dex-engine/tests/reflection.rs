//! Uji reflection: `Class.getMethod` dan `Method.invoke` yang masuk kembali
//! ke interpreter.

use arunika_dex_engine::{DexEngine, Value, parse_bytes};
use arunika_test_fixtures::dex::{
    ACC_PUBLIC, ACC_STATIC, CatchSpec, ClassSpec, MethodSpec, build_dex_with_symbols, insn,
};

fn engine(bytes: &[u8]) -> DexEngine {
    let dex = parse_bytes(bytes, "reflection.dex").expect("DEX fixture harus terparse");
    DexEngine::new(dex)
}

fn reflection_classes() -> Vec<ClassSpec> {
    vec![
        ClassSpec::new("Ljava/lang/Class;", Some("Ljava/lang/Object;")).with_method(
            MethodSpec::new(
                "getMethod",
                ACC_PUBLIC,
                &["Ljava/lang/String;", "[Ljava/lang/Class;"],
                "Ljava/lang/reflect/Method;",
            ),
        ),
        ClassSpec::new("Ljava/lang/reflect/Method;", Some("Ljava/lang/Object;")).with_method(
            MethodSpec::new(
                "invoke",
                ACC_PUBLIC,
                &["Ljava/lang/Object;", "[Ljava/lang/Object;"],
                "Ljava/lang/Object;",
            ),
        ),
        ClassSpec::new("LTarget;", Some("Ljava/lang/Object;")).with_method(
            MethodSpec::new("tambah", ACC_PUBLIC | ACC_STATIC, &["I", "I"], "I")
                .registers(3)
                .ins(2)
                .body([insn::const_16(2, 42), insn::return_(2)].concat()),
        ),
    ]
}

#[test]
fn get_method_lalu_invoke_memanggil_method_dex() {
    let build = |get_method: u16,
                 invoke: u16,
                 int_type: u16,
                 class_array: u16,
                 object_array: u16,
                 target_type: u16,
                 name_index: u16| {
        let caller = MethodSpec::new("pakai", ACC_PUBLIC | ACC_STATIC, &[], "Ljava/lang/Object;")
            .registers(11)
            .body(
                [
                    insn::const_class(0, int_type),
                    insn::const_class(1, int_type),
                    insn::filled_new_array(class_array, &[0, 1]),
                    insn::move_result(2),
                    insn::const_class(3, target_type),
                    insn::const_string(4, name_index),
                    insn::invoke_virtual(get_method, &[3, 4, 2]),
                    insn::move_result(5),
                    insn::const_16(6, 20),
                    insn::const_16(7, 22),
                    insn::filled_new_array(object_array, &[6, 7]),
                    insn::move_result(8),
                    insn::const_4(9, 0),
                    insn::invoke_virtual(invoke, &[5, 9, 8]),
                    insn::move_result(10),
                    insn::return_object(10),
                ]
                .concat(),
            )
            .literal("tambah");

        let mut classes = reflection_classes();
        classes.push(ClassSpec::new("LContoh;", Some("Ljava/lang/Object;")).with_method(caller));
        build_dex_with_symbols(&classes)
    };

    let probe = build(0, 0, 0, 0, 0, 0, 0);
    let get_method = probe.method_index("Ljava/lang/Class;", "getMethod");
    let invoke = probe.method_index("Ljava/lang/reflect/Method;", "invoke");
    let int_type = probe.type_index("I");
    let class_array = probe.type_index("[Ljava/lang/Class;");
    let object_array = probe.type_index("[Ljava/lang/Object;");
    let target_type = probe.type_index("LTarget;");
    let name_index = probe.string_index("tambah");

    let bytes = build(
        get_method,
        invoke,
        int_type,
        class_array,
        object_array,
        target_type,
        name_index,
    )
    .bytes;

    let mut engine = engine(&bytes);
    let outcome = engine
        .execute_method("LContoh;", "pakai")
        .expect("reflection harus berhasil");
    assert_eq!(outcome.return_value, Some(Value::Int(42)));
}

#[test]
fn get_method_yang_tidak_ada_melempar_nosuchmethodexception() {
    let build = |get_method: u16, target_type: u16, name_index: u16| {
        let caller = MethodSpec::new("cari", ACC_PUBLIC | ACC_STATIC, &[], "I")
            .registers(4)
            .body(
                [
                    insn::const_class(0, target_type),
                    insn::const_string(1, name_index),
                    insn::const_4(2, 0),
                    insn::invoke_virtual(get_method, &[0, 1, 2]),
                    insn::move_result(3),
                    insn::move_exception(3),
                    insn::const_4(0, 1),
                    insn::return_(0),
                ]
                .concat(),
            )
            .try_catch(
                5,
                3,
                CatchSpec::typed("Ljava/lang/NoSuchMethodException;", 9),
            )
            .literal("tidakAda");

        let mut classes = reflection_classes();
        classes.push(ClassSpec::new("LContoh;", Some("Ljava/lang/Object;")).with_method(caller));
        build_dex_with_symbols(&classes)
    };

    let probe = build(0, 0, 0);
    let get_method = probe.method_index("Ljava/lang/Class;", "getMethod");
    let target_type = probe.type_index("LTarget;");
    let name_index = probe.string_index("tidakAda");
    let bytes = build(get_method, target_type, name_index).bytes;

    let mut engine = engine(&bytes);
    let outcome = engine
        .execute_method("LContoh;", "cari")
        .expect("eksepsi harus tertangkap");
    assert_eq!(outcome.return_value, Some(Value::Int(1)));
}
