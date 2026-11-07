//! Uji integrasi resolver framework dengan interpreter DEX.

use arunika_dex_engine::{DexEngine, Value, parse_bytes};
use arunika_framework_api::AndroidNativeMethods;
use arunika_framework_api::AppManifest;
use arunika_sandbox_manager::ConfinedFileSystem;
use arunika_test_fixtures::dex::{
    ACC_PUBLIC, ACC_STATIC, CatchSpec, ClassSpec, FieldSpec, MethodSpec, build_dex_with_symbols,
    insn,
};

#[test]
fn bytecode_memanggil_math_abs_milik_framework() {
    let build = |method_index: u16| {
        let math = ClassSpec::new("Ljava/lang/Math;", None).with_method(MethodSpec::new(
            "abs",
            ACC_PUBLIC | ACC_STATIC,
            &["I"],
            "I",
        ));
        let caller = MethodSpec::new("panggil", ACC_PUBLIC | ACC_STATIC, &[], "I")
            .registers(1)
            .body(
                [
                    insn::const_16(0, -21),
                    insn::invoke_static(method_index, &[0]),
                    insn::move_result(0),
                    insn::return_(0),
                ]
                .concat(),
            );
        build_dex_with_symbols(&[math, ClassSpec::new("LContoh;", None).with_method(caller)])
    };

    let probe = build(0);
    let index = probe.method_index("Ljava/lang/Math;", "abs");
    let bytes = build(index).bytes;

    let dex = parse_bytes(&bytes, "uji.dex").expect("DEX fixture harus terparse");
    let mut engine = DexEngine::new(dex);
    engine.set_native_resolver(Box::new(AndroidNativeMethods::new()));

    let outcome = engine
        .execute_method("LContoh;", "panggil")
        .expect("eksekusi harus berhasil");
    assert_eq!(outcome.return_value, Some(Value::Int(21)));
}

#[test]
fn bytecode_memanggil_string_length_dengan_akses_heap() {
    let build = |length_index: u16, string_index: u16| {
        let string = ClassSpec::new("Ljava/lang/String;", Some("Ljava/lang/Object;"))
            .with_method(MethodSpec::new("length", ACC_PUBLIC, &[], "I"));
        let caller = MethodSpec::new("panjang", ACC_PUBLIC | ACC_STATIC, &[], "I")
            .registers(2)
            .ins(0)
            .body(
                [
                    insn::const_string(0, string_index),
                    insn::invoke_virtual(length_index, &[0]),
                    insn::move_result(1),
                    insn::return_(1),
                ]
                .concat(),
            )
            .literal("halo");
        build_dex_with_symbols(&[string, ClassSpec::new("LContoh;", None).with_method(caller)])
    };

    let probe = build(0, 0);
    let length_index = probe.method_index("Ljava/lang/String;", "length");
    let string_index = probe.string_index("halo");
    let bytes = build(length_index, string_index).bytes;

    let dex = parse_bytes(&bytes, "uji.dex").expect("DEX fixture harus terparse");
    let mut engine = DexEngine::new(dex);
    engine.set_native_resolver(Box::new(AndroidNativeMethods::new()));

    let outcome = engine
        .execute_method("LContoh;", "panjang")
        .expect("eksekusi harus berhasil");
    assert_eq!(outcome.return_value, Some(Value::Int(4)));
}

#[test]
fn bytecode_parse_int_mengubah_string_menjadi_int() {
    let build = |parse_index: u16, string_index: u16| {
        let integer = ClassSpec::new("Ljava/lang/Integer;", Some("Ljava/lang/Object;"))
            .with_method(MethodSpec::new(
                "parseInt",
                ACC_PUBLIC | ACC_STATIC,
                &["Ljava/lang/String;"],
                "I",
            ));
        let caller = MethodSpec::new("ubah", ACC_PUBLIC | ACC_STATIC, &[], "I")
            .registers(2)
            .ins(0)
            .body(
                [
                    insn::const_string(0, string_index),
                    insn::invoke_static(parse_index, &[0]),
                    insn::move_result(1),
                    insn::return_(1),
                ]
                .concat(),
            )
            .literal("123");
        build_dex_with_symbols(&[
            integer,
            ClassSpec::new("LContoh;", None).with_method(caller),
        ])
    };

    let probe = build(0, 0);
    let parse_index = probe.method_index("Ljava/lang/Integer;", "parseInt");
    let string_index = probe.string_index("123");
    let bytes = build(parse_index, string_index).bytes;

    let dex = parse_bytes(&bytes, "uji.dex").expect("DEX fixture harus terparse");
    let mut engine = DexEngine::new(dex);
    engine.set_native_resolver(Box::new(AndroidNativeMethods::new()));

    let outcome = engine
        .execute_method("LContoh;", "ubah")
        .expect("eksekusi harus berhasil");
    assert_eq!(outcome.return_value, Some(Value::Int(123)));
}

#[test]
fn math_abs_long_memakai_overload_berdasarkan_signature() {
    let build = |abs_index: u16| {
        let math = ClassSpec::new("Ljava/lang/Math;", Some("Ljava/lang/Object;"))
            .with_method(MethodSpec::new("abs", ACC_PUBLIC | ACC_STATIC, &["J"], "J"));
        let caller = MethodSpec::new("mutlak", ACC_PUBLIC | ACC_STATIC, &[], "J")
            .registers(2)
            .ins(0)
            .body(
                [
                    insn::const_wide_16(0, -5),
                    insn::invoke_static(abs_index, &[0, 1]),
                    insn::move_result(0),
                    insn::return_wide(0),
                ]
                .concat(),
            );
        build_dex_with_symbols(&[math, ClassSpec::new("LContoh;", None).with_method(caller)])
    };

    let abs_index = build(0).method_index("Ljava/lang/Math;", "abs");
    let bytes = build(abs_index).bytes;

    let dex = parse_bytes(&bytes, "uji.dex").expect("DEX fixture harus terparse");
    let mut engine = DexEngine::new(dex);
    engine.set_native_resolver(Box::new(AndroidNativeMethods::new()));

    let outcome = engine
        .execute_method("LContoh;", "mutlak")
        .expect("eksekusi harus berhasil");
    assert_eq!(outcome.return_value, Some(Value::from_long(5)));
}

fn method_index(
    dex: &arunika_dex_engine::DexFile,
    class: &str,
    name: &str,
    params: &[&str],
) -> u16 {
    dex.method_ids
        .iter()
        .position(|method| {
            method.class == class
                && method.name == name
                && dex.proto_ids[method.proto_idx as usize].parameter_types == params
        })
        .expect("method dengan proto tersebut harus ada") as u16
}

#[test]
fn string_builder_dirangkai_dan_dikonversi_ke_string() {
    let build = |builder_type: u16,
                 init: u16,
                 append_text: u16,
                 append_int: u16,
                 to_string: u16,
                 length: u16,
                 text: u16| {
        let builder = ClassSpec::new("Ljava/lang/StringBuilder;", Some("Ljava/lang/Object;"))
            .with_method(MethodSpec::new("<init>", ACC_PUBLIC | 0x1_0000, &[], "V"))
            .with_method(MethodSpec::new(
                "append",
                ACC_PUBLIC,
                &["Ljava/lang/String;"],
                "Ljava/lang/StringBuilder;",
            ))
            .with_method(MethodSpec::new(
                "append",
                ACC_PUBLIC,
                &["I"],
                "Ljava/lang/StringBuilder;",
            ))
            .with_method(MethodSpec::new(
                "toString",
                ACC_PUBLIC,
                &[],
                "Ljava/lang/String;",
            ));
        let string = ClassSpec::new("Ljava/lang/String;", Some("Ljava/lang/Object;"))
            .with_method(MethodSpec::new("length", ACC_PUBLIC, &[], "I"));
        let caller = MethodSpec::new("jalankan", ACC_PUBLIC | ACC_STATIC, &[], "I")
            .registers(5)
            .ins(0)
            .body(
                [
                    insn::new_instance(0, builder_type),
                    insn::invoke_direct(init, &[0]),
                    insn::const_string(1, text),
                    insn::invoke_virtual(append_text, &[0, 1]),
                    insn::const_4(2, 7),
                    insn::invoke_virtual(append_int, &[0, 2]),
                    insn::invoke_virtual(to_string, &[0]),
                    insn::move_result(3),
                    insn::invoke_virtual(length, &[3]),
                    insn::move_result(4),
                    insn::return_(4),
                ]
                .concat(),
            )
            .literal("ha");
        build_dex_with_symbols(&[
            builder,
            string,
            ClassSpec::new("LContoh;", None).with_method(caller),
        ])
    };

    let probe = build(0, 0, 0, 0, 0, 0, 0);
    let dex = parse_bytes(&probe.bytes, "uji.dex").expect("DEX fixture harus terparse");
    let builder_type = probe.type_index("Ljava/lang/StringBuilder;");
    let text = probe.string_index("ha");
    let init = method_index(&dex, "Ljava/lang/StringBuilder;", "<init>", &[]);
    let append_text = method_index(
        &dex,
        "Ljava/lang/StringBuilder;",
        "append",
        &["Ljava/lang/String;"],
    );
    let append_int = method_index(&dex, "Ljava/lang/StringBuilder;", "append", &["I"]);
    let to_string = method_index(&dex, "Ljava/lang/StringBuilder;", "toString", &[]);
    let length = method_index(&dex, "Ljava/lang/String;", "length", &[]);

    let bytes = build(
        builder_type,
        init,
        append_text,
        append_int,
        to_string,
        length,
        text,
    )
    .bytes;
    let dex = parse_bytes(&bytes, "uji.dex").expect("DEX fixture harus terparse");
    let mut engine = DexEngine::new(dex);
    engine.set_native_resolver(Box::new(AndroidNativeMethods::new()));

    let outcome = engine
        .execute_method("LContoh;", "jalankan")
        .expect("eksekusi harus berhasil");
    // "ha" + "7" = 3 karakter.
    assert_eq!(outcome.return_value, Some(Value::Int(3)));
}

#[test]
fn system_arraycopy_menyalin_elemen_array() {
    let build = |arraycopy: u16, int_type: u16| {
        let system = ClassSpec::new("Ljava/lang/System;", Some("Ljava/lang/Object;")).with_method(
            MethodSpec::new(
                "arraycopy",
                ACC_PUBLIC | ACC_STATIC,
                &["Ljava/lang/Object;", "I", "Ljava/lang/Object;", "I", "I"],
                "V",
            ),
        );
        let caller = MethodSpec::new("salin", ACC_PUBLIC | ACC_STATIC, &[], "I")
            .registers(10)
            .body(
                [
                    insn::const_4(0, 3),
                    insn::new_array(1, 0, int_type),
                    insn::new_array(2, 0, int_type),
                    insn::const_4(3, 0),
                    insn::const_16(4, 42),
                    insn::aput(4, 1, 3),
                    insn::const_4(5, 0),
                    insn::const_4(6, 0),
                    insn::const_4(7, 3),
                    insn::invoke_static(arraycopy, &[1, 5, 2, 6, 7]),
                    insn::const_4(8, 0),
                    insn::aget(9, 2, 8),
                    insn::return_(9),
                ]
                .concat(),
            );
        build_dex_with_symbols(&[system, ClassSpec::new("LContoh;", None).with_method(caller)])
    };

    let probe = build(0, 0);
    let arraycopy = probe.method_index("Ljava/lang/System;", "arraycopy");
    let int_type = probe.type_index("I");
    let bytes = build(arraycopy, int_type).bytes;

    let dex = parse_bytes(&bytes, "uji.dex").expect("DEX fixture harus terparse");
    let mut engine = DexEngine::new(dex);
    engine.set_native_resolver(Box::new(AndroidNativeMethods::new()));

    let outcome = engine
        .execute_method("LContoh;", "salin")
        .expect("eksekusi harus berhasil");
    assert_eq!(outcome.return_value, Some(Value::Int(42)));
}

#[test]
fn instantiate_class_memanggil_konstruktor_induk_activity() {
    let build = |activity_init: u16| {
        let activity = ClassSpec::new("Landroid/app/Activity;", Some("Ljava/lang/Object;"))
            .with_method(MethodSpec::new("<init>", ACC_PUBLIC | 0x1_0000, &[], "V"));
        let launcher = ClassSpec::new("LContohActivity;", Some("Landroid/app/Activity;"))
            .with_method(
                MethodSpec::new("<init>", ACC_PUBLIC | 0x1_0000, &[], "V")
                    .registers(1)
                    .ins(1)
                    .body(
                        [
                            insn::invoke_direct(activity_init, &[0]),
                            insn::return_void(),
                        ]
                        .concat(),
                    ),
            );
        build_dex_with_symbols(&[activity, launcher, ClassSpec::new("LContoh;", None)])
    };

    let probe = build(0);
    let activity_init = probe.method_index("Landroid/app/Activity;", "<init>");
    let bytes = build(activity_init).bytes;

    let dex = parse_bytes(&bytes, "uji.dex").expect("DEX fixture harus terparse");
    let mut engine = DexEngine::new(dex);
    engine.set_native_resolver(Box::new(AndroidNativeMethods::new()));

    let value = engine
        .instantiate_class("LContohActivity;")
        .expect("instance activity harus dibuat");
    assert!(matches!(value, Value::Ref(_)));
}

#[test]
fn const_class_lalu_get_name_mengembalikan_nama_java() {
    let build = |get_name: u16, string_type: u16| {
        let class_spec =
            ClassSpec::new("Ljava/lang/Class;", Some("Ljava/lang/Object;")).with_method(
                MethodSpec::new("getName", ACC_PUBLIC, &[], "Ljava/lang/String;"),
            );
        let caller = MethodSpec::new("nama", ACC_PUBLIC | ACC_STATIC, &[], "Ljava/lang/String;")
            .registers(2)
            .body(
                [
                    insn::const_class(0, string_type),
                    insn::invoke_virtual(get_name, &[0]),
                    insn::move_result(1),
                    insn::return_object(1),
                ]
                .concat(),
            );
        build_dex_with_symbols(&[
            class_spec,
            ClassSpec::new("LContoh;", None).with_method(caller),
        ])
    };

    let probe = build(0, 0);
    let get_name = probe.method_index("Ljava/lang/Class;", "getName");
    let string_type = probe.type_index("Ljava/lang/String;");
    let bytes = build(get_name, string_type).bytes;

    let dex = parse_bytes(&bytes, "uji.dex").expect("DEX fixture harus terparse");
    let mut engine = DexEngine::new(dex);
    engine.set_native_resolver(Box::new(AndroidNativeMethods::new()));

    let outcome = engine
        .execute_method("LContoh;", "nama")
        .expect("eksekusi harus berhasil");
    let reference = outcome
        .return_value
        .and_then(Value::as_ref)
        .expect("getName harus mengembalikan String");
    assert_eq!(engine.heap.string(reference), Some("java.lang.String"));
}

#[test]
fn arraylist_menyimpan_dan_mengambil_nilai_boxed() {
    let build = |init: u16, add: u16, get: u16, value_of: u16, int_value: u16, list_type: u16| {
        let list = ClassSpec::new("Ljava/util/ArrayList;", Some("Ljava/lang/Object;"))
            .with_method(MethodSpec::new("<init>", ACC_PUBLIC | 0x1_0000, &[], "V"))
            .with_method(MethodSpec::new(
                "add",
                ACC_PUBLIC,
                &["Ljava/lang/Object;"],
                "Z",
            ))
            .with_method(MethodSpec::new(
                "get",
                ACC_PUBLIC,
                &["I"],
                "Ljava/lang/Object;",
            ));
        let integer = ClassSpec::new("Ljava/lang/Integer;", Some("Ljava/lang/Object;"))
            .with_method(MethodSpec::new(
                "valueOf",
                ACC_PUBLIC | ACC_STATIC,
                &["I"],
                "Ljava/lang/Integer;",
            ))
            .with_method(MethodSpec::new("intValue", ACC_PUBLIC, &[], "I"));
        let caller = MethodSpec::new("jalankan", ACC_PUBLIC | ACC_STATIC, &[], "I")
            .registers(6)
            .body(
                [
                    insn::new_instance(0, list_type),
                    insn::invoke_direct(init, &[0]),
                    insn::const_16(1, 42),
                    insn::invoke_static(value_of, &[1]),
                    insn::move_result(2),
                    insn::invoke_virtual(add, &[0, 2]),
                    insn::const_4(3, 0),
                    insn::invoke_virtual(get, &[0, 3]),
                    insn::move_result(4),
                    insn::invoke_virtual(int_value, &[4]),
                    insn::move_result(5),
                    insn::return_(5),
                ]
                .concat(),
            );
        build_dex_with_symbols(&[
            list,
            integer,
            ClassSpec::new("LContoh;", None).with_method(caller),
        ])
    };

    let probe = build(0, 0, 0, 0, 0, 0);
    let probe_dex = parse_bytes(&probe.bytes, "probe.dex").expect("probe harus terparse");
    let list_type = probe.type_index("Ljava/util/ArrayList;");
    let init = method_index(&probe_dex, "Ljava/util/ArrayList;", "<init>", &[]);
    let add = method_index(
        &probe_dex,
        "Ljava/util/ArrayList;",
        "add",
        &["Ljava/lang/Object;"],
    );
    let get = method_index(&probe_dex, "Ljava/util/ArrayList;", "get", &["I"]);
    let value_of = method_index(&probe_dex, "Ljava/lang/Integer;", "valueOf", &["I"]);
    let int_value = method_index(&probe_dex, "Ljava/lang/Integer;", "intValue", &[]);

    let bytes = build(init, add, get, value_of, int_value, list_type).bytes;
    let dex = parse_bytes(&bytes, "uji.dex").expect("DEX fixture harus terparse");
    let mut engine = DexEngine::new(dex);
    engine.set_native_resolver(Box::new(AndroidNativeMethods::new()));

    let outcome = engine
        .execute_method("LContoh;", "jalankan")
        .expect("eksekusi harus berhasil");
    assert_eq!(outcome.return_value, Some(Value::Int(42)));
}

#[test]
fn hashmap_mencocokkan_kunci_string_berdasarkan_isi() {
    let build =
        |init: u16, put: u16, get: u16, length: u16, map_type: u16, kunci: u16, nilai: u16| {
            let map = ClassSpec::new("Ljava/util/HashMap;", Some("Ljava/lang/Object;"))
                .with_method(MethodSpec::new("<init>", ACC_PUBLIC | 0x1_0000, &[], "V"))
                .with_method(MethodSpec::new(
                    "put",
                    ACC_PUBLIC,
                    &["Ljava/lang/Object;", "Ljava/lang/Object;"],
                    "Ljava/lang/Object;",
                ))
                .with_method(MethodSpec::new(
                    "get",
                    ACC_PUBLIC,
                    &["Ljava/lang/Object;"],
                    "Ljava/lang/Object;",
                ));
            let string = ClassSpec::new("Ljava/lang/String;", Some("Ljava/lang/Object;"))
                .with_method(MethodSpec::new("length", ACC_PUBLIC, &[], "I"));
            let caller = MethodSpec::new("jalankan", ACC_PUBLIC | ACC_STATIC, &[], "I")
                .registers(6)
                .body(
                    [
                        insn::new_instance(0, map_type),
                        insn::invoke_direct(init, &[0]),
                        insn::const_string(1, kunci),
                        insn::const_string(2, nilai),
                        insn::invoke_virtual(put, &[0, 1, 2]),
                        insn::const_string(3, kunci),
                        insn::invoke_virtual(get, &[0, 3]),
                        insn::move_result(4),
                        insn::invoke_virtual(length, &[4]),
                        insn::move_result(5),
                        insn::return_(5),
                    ]
                    .concat(),
                )
                .literal("kunci")
                .literal("nilai");
            build_dex_with_symbols(&[
                map,
                string,
                ClassSpec::new("LContoh;", None).with_method(caller),
            ])
        };

    let probe = build(0, 0, 0, 0, 0, 0, 0);
    let probe_dex = parse_bytes(&probe.bytes, "probe.dex").expect("probe harus terparse");
    let map_type = probe.type_index("Ljava/util/HashMap;");
    let init = method_index(&probe_dex, "Ljava/util/HashMap;", "<init>", &[]);
    let put = method_index(
        &probe_dex,
        "Ljava/util/HashMap;",
        "put",
        &["Ljava/lang/Object;", "Ljava/lang/Object;"],
    );
    let get = method_index(
        &probe_dex,
        "Ljava/util/HashMap;",
        "get",
        &["Ljava/lang/Object;"],
    );
    let length = method_index(&probe_dex, "Ljava/lang/String;", "length", &[]);
    let kunci = probe.string_index("kunci");
    let nilai = probe.string_index("nilai");

    let bytes = build(init, put, get, length, map_type, kunci, nilai).bytes;
    let dex = parse_bytes(&bytes, "uji.dex").expect("DEX fixture harus terparse");
    let mut engine = DexEngine::new(dex);
    engine.set_native_resolver(Box::new(AndroidNativeMethods::new()));

    let outcome = engine
        .execute_method("LContoh;", "jalankan")
        .expect("eksekusi harus berhasil");
    // Nilai untuk kunci "kunci" adalah "nilai" (5 karakter).
    assert_eq!(outcome.return_value, Some(Value::Int(5)));
}

#[test]
fn invoke_range_memanggil_method_statis_dan_virtual() {
    let build = |abs: u16, length: u16, text: u16| {
        let math = ClassSpec::new("Ljava/lang/Math;", None).with_method(MethodSpec::new(
            "abs",
            ACC_PUBLIC | ACC_STATIC,
            &["I"],
            "I",
        ));
        let string = ClassSpec::new("Ljava/lang/String;", Some("Ljava/lang/Object;"))
            .with_method(MethodSpec::new("length", ACC_PUBLIC, &[], "I"));
        let caller = MethodSpec::new("jalankan", ACC_PUBLIC | ACC_STATIC, &[], "I")
            .registers(6)
            .body(
                [
                    insn::const_16(2, -21),
                    insn::invoke_static_range(abs, 2, 1),
                    insn::move_result(3),
                    insn::const_string(0, text),
                    insn::invoke_virtual_range(length, 0, 1),
                    insn::move_result(4),
                    insn::add_int(5, 3, 4),
                    insn::return_(5),
                ]
                .concat(),
            )
            .literal("halo");
        build_dex_with_symbols(&[
            math,
            string,
            ClassSpec::new("LContoh;", None).with_method(caller),
        ])
    };

    let probe = build(0, 0, 0);
    let probe_dex = parse_bytes(&probe.bytes, "probe.dex").expect("probe harus terparse");
    let abs = method_index(&probe_dex, "Ljava/lang/Math;", "abs", &["I"]);
    let length = method_index(&probe_dex, "Ljava/lang/String;", "length", &[]);
    let text = probe.string_index("halo");

    let bytes = build(abs, length, text).bytes;
    let dex = parse_bytes(&bytes, "uji.dex").expect("DEX fixture harus terparse");
    let mut engine = DexEngine::new(dex);
    engine.set_native_resolver(Box::new(AndroidNativeMethods::new()));

    let outcome = engine
        .execute_method("LContoh;", "jalankan")
        .expect("eksekusi harus berhasil");
    // 21 + panjang "halo" (4).
    assert_eq!(outcome.return_value, Some(Value::Int(25)));
}
#[test]
fn package_manager_membaca_version_name_dari_manifest() {
    let build = |get_pm: u16,
                 get_info: u16,
                 version_field: u16,
                 package_text: u16,
                 activity: u16| {
        let activity_spec = ClassSpec::new("Landroid/app/Activity;", Some("Ljava/lang/Object;"))
            .with_method(MethodSpec::new(
                "getPackageManager",
                ACC_PUBLIC,
                &[],
                "Landroid/content/pm/PackageManager;",
            ));
        let pm_spec = ClassSpec::new(
            "Landroid/content/pm/PackageManager;",
            Some("Ljava/lang/Object;"),
        )
        .with_method(MethodSpec::new(
            "getPackageInfo",
            ACC_PUBLIC,
            &["Ljava/lang/String;", "I"],
            "Landroid/content/pm/PackageInfo;",
        ));
        let info_spec = ClassSpec::new(
            "Landroid/content/pm/PackageInfo;",
            Some("Ljava/lang/Object;"),
        )
        .with_field(FieldSpec::new("versionName", "Ljava/lang/String;", false));
        let caller = MethodSpec::new("versi", ACC_PUBLIC | ACC_STATIC, &[], "Ljava/lang/String;")
            .registers(5)
            .body(
                [
                    insn::new_instance(0, activity),
                    insn::invoke_virtual(get_pm, &[0]),
                    insn::move_result(1),
                    insn::const_string(2, package_text),
                    insn::const_4(3, 0),
                    insn::invoke_virtual(get_info, &[1, 2, 3]),
                    insn::move_result(4),
                    insn::iget_object(0, 4, version_field),
                    insn::return_object(0),
                ]
                .concat(),
            )
            .literal("com.contoh");

        build_dex_with_symbols(&[
            activity_spec,
            pm_spec,
            info_spec,
            ClassSpec::new("LContoh;", None).with_method(caller),
        ])
    };

    let probe = build(0, 0, 0, 0, 0);
    let get_pm = probe.method_index("Landroid/app/Activity;", "getPackageManager");
    let get_info = probe.method_index("Landroid/content/pm/PackageManager;", "getPackageInfo");
    let version_field = probe.field_index("Landroid/content/pm/PackageInfo;", "versionName");
    let package_text = probe.string_index("com.contoh");
    let activity = probe.type_index("Landroid/app/Activity;");
    let bytes = build(get_pm, get_info, version_field, package_text, activity).bytes;

    let dex = parse_bytes(&bytes, "uji.dex").expect("DEX fixture harus terparse");
    let mut engine = DexEngine::new(dex);
    engine.set_native_resolver(Box::new(AndroidNativeMethods::new().with_manifest(
        AppManifest {
            package: Some("com.contoh".to_string()),
            version_name: Some("2.4.1".to_string()),
            version_code: Some(20401),
            target_sdk: Some(33),
            min_sdk: Some(21),
            label: Some("Contoh".to_string()),
            meta_data: vec![("com.contoh.channel".to_string(), "beta".to_string())],
            permissions: Vec::new(),
        },
    )));

    let outcome = engine
        .execute_method("LContoh;", "versi")
        .expect("eksekusi harus berhasil");
    let reference = outcome
        .return_value
        .and_then(Value::as_ref)
        .expect("versionName harus mengembalikan String");
    assert_eq!(engine.heap.string(reference), Some("2.4.1"));
}
#[test]
fn bundle_menyimpan_dan_mengembalikan_nilai_string() {
    let build = |init: u16, put: u16, get: u16, bundle_type: u16, key: u16, value: u16| {
        let bundle = ClassSpec::new("Landroid/os/Bundle;", Some("Ljava/lang/Object;"))
            .with_method(MethodSpec::new("<init>", ACC_PUBLIC, &[], "V"))
            .with_method(MethodSpec::new(
                "putString",
                ACC_PUBLIC,
                &["Ljava/lang/String;", "Ljava/lang/String;"],
                "V",
            ))
            .with_method(MethodSpec::new(
                "getString",
                ACC_PUBLIC,
                &["Ljava/lang/String;"],
                "Ljava/lang/String;",
            ));
        let caller = MethodSpec::new("bulat", ACC_PUBLIC | ACC_STATIC, &[], "Ljava/lang/String;")
            .registers(3)
            .body(
                [
                    insn::new_instance(0, bundle_type),
                    insn::invoke_virtual(init, &[0]),
                    insn::const_string(1, key),
                    insn::const_string(2, value),
                    insn::invoke_virtual(put, &[0, 1, 2]),
                    insn::invoke_virtual(get, &[0, 1]),
                    insn::move_result(0),
                    insn::return_object(0),
                ]
                .concat(),
            )
            .literal("kunci")
            .literal("nilai");

        build_dex_with_symbols(&[bundle, ClassSpec::new("LContoh;", None).with_method(caller)])
    };

    let probe = build(0, 0, 0, 0, 0, 0);
    let init = probe.method_index("Landroid/os/Bundle;", "<init>");
    let put = probe.method_index("Landroid/os/Bundle;", "putString");
    let get = probe.method_index("Landroid/os/Bundle;", "getString");
    let bundle_type = probe.type_index("Landroid/os/Bundle;");
    let key = probe.string_index("kunci");
    let value = probe.string_index("nilai");
    let bytes = build(init, put, get, bundle_type, key, value).bytes;

    let dex = parse_bytes(&bytes, "uji.dex").expect("DEX fixture harus terparse");
    let mut engine = DexEngine::new(dex);
    engine.set_native_resolver(Box::new(AndroidNativeMethods::new()));

    let outcome = engine
        .execute_method("LContoh;", "bulat")
        .expect("eksekusi harus berhasil");
    let reference = outcome
        .return_value
        .and_then(Value::as_ref)
        .expect("getString harus mengembalikan String");
    assert_eq!(engine.heap.string(reference), Some("nilai"));
}
#[test]
fn application_info_meta_data_terbaca_sebagai_bundle() {
    let build = |get_pm: u16,
                 get_info: u16,
                 get_string: u16,
                 meta_field: u16,
                 activity: u16,
                 pkg: u16,
                 key: u16| {
        let activity_spec = ClassSpec::new("Landroid/app/Activity;", Some("Ljava/lang/Object;"))
            .with_method(MethodSpec::new(
                "getPackageManager",
                ACC_PUBLIC,
                &[],
                "Landroid/content/pm/PackageManager;",
            ));
        let pm_spec = ClassSpec::new(
            "Landroid/content/pm/PackageManager;",
            Some("Ljava/lang/Object;"),
        )
        .with_method(MethodSpec::new(
            "getApplicationInfo",
            ACC_PUBLIC,
            &["Ljava/lang/String;", "I"],
            "Landroid/content/pm/ApplicationInfo;",
        ));
        let info_spec = ClassSpec::new(
            "Landroid/content/pm/ApplicationInfo;",
            Some("Ljava/lang/Object;"),
        )
        .with_field(FieldSpec::new("metaData", "Landroid/os/Bundle;", false));
        let bundle_spec = ClassSpec::new("Landroid/os/Bundle;", Some("Ljava/lang/Object;"))
            .with_method(MethodSpec::new(
                "getString",
                ACC_PUBLIC,
                &["Ljava/lang/String;"],
                "Ljava/lang/String;",
            ));
        let caller = MethodSpec::new("kanal", ACC_PUBLIC | ACC_STATIC, &[], "Ljava/lang/String;")
            .registers(5)
            .body(
                [
                    insn::new_instance(0, activity),
                    insn::invoke_virtual(get_pm, &[0]),
                    insn::move_result(1),
                    insn::const_string(2, pkg),
                    insn::const_4(3, 0),
                    insn::invoke_virtual(get_info, &[1, 2, 3]),
                    insn::move_result(4),
                    insn::iget_object(0, 4, meta_field),
                    insn::const_string(2, key),
                    insn::invoke_virtual(get_string, &[0, 2]),
                    insn::move_result(0),
                    insn::return_object(0),
                ]
                .concat(),
            )
            .literal("com.contoh")
            .literal("com.contoh.channel");

        build_dex_with_symbols(&[
            activity_spec,
            pm_spec,
            info_spec,
            bundle_spec,
            ClassSpec::new("LContoh;", None).with_method(caller),
        ])
    };

    let probe = build(0, 0, 0, 0, 0, 0, 0);
    let get_pm = probe.method_index("Landroid/app/Activity;", "getPackageManager");
    let get_info = probe.method_index("Landroid/content/pm/PackageManager;", "getApplicationInfo");
    let get_string = probe.method_index("Landroid/os/Bundle;", "getString");
    let meta_field = probe.field_index("Landroid/content/pm/ApplicationInfo;", "metaData");
    let activity = probe.type_index("Landroid/app/Activity;");
    let pkg = probe.string_index("com.contoh");
    let key = probe.string_index("com.contoh.channel");
    let bytes = build(get_pm, get_info, get_string, meta_field, activity, pkg, key).bytes;

    let dex = parse_bytes(&bytes, "uji.dex").expect("DEX fixture harus terparse");
    let mut engine = DexEngine::new(dex);
    engine.set_native_resolver(Box::new(AndroidNativeMethods::new().with_manifest(
        AppManifest {
            package: Some("com.contoh".to_string()),
            meta_data: vec![("com.contoh.channel".to_string(), "beta".to_string())],
            ..AppManifest::default()
        },
    )));

    let outcome = engine
        .execute_method("LContoh;", "kanal")
        .expect("eksekusi harus berhasil");
    let reference = outcome
        .return_value
        .and_then(Value::as_ref)
        .expect("metaData harus mengembalikan String");
    assert_eq!(engine.heap.string(reference), Some("beta"));
}
/// Berkas aplikasi dalam memori untuk menguji API aliran tanpa menyentuh disk.
#[derive(Debug, Default)]
struct MemoryFileSystem {
    files: std::sync::Mutex<std::collections::HashMap<String, Vec<u8>>>,
}

impl arunika_sandbox_manager::ConfinedFileSystem for MemoryFileSystem {
    fn read(&self, name: &str) -> Result<Vec<u8>, arunika_sandbox_manager::SandboxError> {
        arunika_sandbox_manager::validate_file_name(name)?;
        self.files
            .lock()
            .expect("mutex")
            .get(name)
            .cloned()
            .ok_or_else(|| arunika_sandbox_manager::SandboxError::FileIo {
                name: name.to_string(),
                message: "tidak ada".to_string(),
            })
    }

    fn write(&self, name: &str, data: &[u8]) -> Result<(), arunika_sandbox_manager::SandboxError> {
        arunika_sandbox_manager::validate_file_name(name)?;
        self.files
            .lock()
            .expect("mutex")
            .insert(name.to_string(), data.to_vec());
        Ok(())
    }

    fn remove(&self, name: &str) -> Result<(), arunika_sandbox_manager::SandboxError> {
        arunika_sandbox_manager::validate_file_name(name)?;
        self.files.lock().expect("mutex").remove(name);
        Ok(())
    }

    fn list(&self) -> Result<Vec<String>, arunika_sandbox_manager::SandboxError> {
        let mut names: Vec<String> = self.files.lock().expect("mutex").keys().cloned().collect();
        names.sort();
        Ok(names)
    }

    fn exists(&self, name: &str) -> bool {
        self.files.lock().expect("mutex").contains_key(name)
    }
}

#[test]
fn aliran_berkas_menulis_lalu_membaca_kembali() {
    let build =
        |open_out: u16, write: u16, close: u16, open_in: u16, read: u16, activity_type: u16| {
            let activity = ClassSpec::new("Landroid/app/Activity;", Some("Ljava/lang/Object;"))
                .with_method(MethodSpec::new(
                    "openFileOutput",
                    ACC_PUBLIC,
                    &["Ljava/lang/String;", "I"],
                    "Ljava/io/FileOutputStream;",
                ))
                .with_method(MethodSpec::new(
                    "openFileInput",
                    ACC_PUBLIC,
                    &["Ljava/lang/String;"],
                    "Ljava/io/FileInputStream;",
                ));
            let output =
                ClassSpec::new("Ljava/io/FileOutputStream;", Some("Ljava/io/OutputStream;"))
                    .with_method(MethodSpec::new("write", ACC_PUBLIC, &["I"], "V"))
                    .with_method(MethodSpec::new("close", ACC_PUBLIC, &[], "V"));
            let input = ClassSpec::new("Ljava/io/FileInputStream;", Some("Ljava/io/InputStream;"))
                .with_method(MethodSpec::new("read", ACC_PUBLIC, &[], "I"));
            let caller = MethodSpec::new("simpan", ACC_PUBLIC | ACC_STATIC, &[], "I")
                .registers(5)
                .body(
                    [
                        insn::new_instance(0, activity_type),
                        insn::const_string(1, 0),
                        insn::const_4(2, 0),
                        insn::invoke_virtual(open_out, &[0, 1, 2]),
                        insn::move_result(3),
                        insn::const_16(4, 65),
                        insn::invoke_virtual(write, &[3, 4]),
                        insn::invoke_virtual(close, &[3]),
                        insn::invoke_virtual(open_in, &[0, 1]),
                        insn::move_result(3),
                        insn::invoke_virtual(read, &[3]),
                        insn::move_result(0),
                        insn::return_(0),
                    ]
                    .concat(),
                )
                .literal("catatan.txt");

            build_dex_with_symbols(&[
                activity,
                output,
                input,
                ClassSpec::new("LContoh;", None).with_method(caller),
            ])
        };

    let probe = build(0, 0, 0, 0, 0, 0);
    let activity_type = probe.type_index("Landroid/app/Activity;");
    let open_out = probe.method_index("Landroid/app/Activity;", "openFileOutput");
    let write = probe.method_index("Ljava/io/FileOutputStream;", "write");
    let close = probe.method_index("Ljava/io/FileOutputStream;", "close");
    let open_in = probe.method_index("Landroid/app/Activity;", "openFileInput");
    let read = probe.method_index("Ljava/io/FileInputStream;", "read");
    let bytes = build(open_out, write, close, open_in, read, activity_type).bytes;

    let dex = parse_bytes(&bytes, "uji.dex").expect("DEX fixture harus terparse");
    let mut engine = DexEngine::new(dex);
    engine.set_native_resolver(Box::new(
        AndroidNativeMethods::new()
            .with_file_system(std::sync::Arc::new(MemoryFileSystem::default())),
    ));

    let outcome = engine
        .execute_method("LContoh;", "simpan")
        .expect("eksekusi harus berhasil");
    assert_eq!(outcome.return_value, Some(Value::Int(65)));
}
#[test]
fn open_file_input_yang_gagal_melempar_eksepsi_yang_bisa_ditangkap() {
    let build = |open_in: u16, get_message: u16, activity_type: u16, name_index: u16| {
        let activity = ClassSpec::new("Landroid/app/Activity;", Some("Ljava/lang/Object;"))
            .with_method(MethodSpec::new(
                "openFileInput",
                ACC_PUBLIC,
                &["Ljava/lang/String;"],
                "Ljava/io/FileInputStream;",
            ));
        let throwable =
            ClassSpec::new("Ljava/lang/Throwable;", Some("Ljava/lang/Object;")).with_method(
                MethodSpec::new("getMessage", ACC_PUBLIC, &[], "Ljava/lang/String;"),
            );
        let caller = MethodSpec::new("pesan", ACC_PUBLIC | ACC_STATIC, &[], "Ljava/lang/String;")
            .registers(3)
            .body(
                [
                    insn::new_instance(0, activity_type),
                    insn::const_string(1, name_index),
                    insn::invoke_virtual(open_in, &[0, 1]),
                    insn::move_result(2),
                    insn::move_exception(2),
                    insn::invoke_virtual(get_message, &[2]),
                    insn::move_result(0),
                    insn::return_object(0),
                ]
                .concat(),
            )
            .try_catch(4, 3, CatchSpec::typed("Ljava/io/FileNotFoundException;", 8))
            .literal("hilang.txt");

        build_dex_with_symbols(&[
            activity,
            throwable,
            ClassSpec::new("LContoh;", Some("Ljava/lang/Object;")).with_method(caller),
        ])
    };

    let probe = build(0, 0, 0, 0);
    let activity_type = probe.type_index("Landroid/app/Activity;");
    let open_in = probe.method_index("Landroid/app/Activity;", "openFileInput");
    let get_message = probe.method_index("Ljava/lang/Throwable;", "getMessage");
    let name_index = probe.string_index("hilang.txt");
    let bytes = build(open_in, get_message, activity_type, name_index).bytes;

    let dex = parse_bytes(&bytes, "uji.dex").expect("DEX fixture harus terparse");
    let mut engine = DexEngine::new(dex);
    engine.set_native_resolver(Box::new(
        AndroidNativeMethods::new()
            .with_file_system(std::sync::Arc::new(MemoryFileSystem::default())),
    ));

    let outcome = engine
        .execute_method("LContoh;", "pesan")
        .expect("eksepsi harus tertangkap");
    let reference = outcome
        .return_value
        .and_then(Value::as_ref)
        .expect("getMessage harus mengembalikan String");
    let message = engine.heap.string(reference).expect("pesan String");
    assert!(
        message.starts_with("hilang.txt: "),
        "pesan tak terduga: {message}"
    );
}

#[test]
fn file_reader_dan_file_writer_membaca_menulis_teks() {
    let build = |reader: u16,
                 buffered: u16,
                 writer: u16,
                 reader_init: u16,
                 buffered_init: u16,
                 read_line: u16,
                 writer_init: u16,
                 write: u16,
                 close: u16,
                 path: u16,
                 text: u16| {
        let classes = vec![
            ClassSpec::new("Ljava/io/FileReader;", Some("Ljava/io/Reader;"))
                .with_method(MethodSpec::new(
                    "<init>",
                    ACC_PUBLIC,
                    &["Ljava/lang/String;"],
                    "V",
                ))
                .with_method(MethodSpec::new(
                    "readLine",
                    ACC_PUBLIC,
                    &[],
                    "Ljava/lang/String;",
                ))
                .with_method(MethodSpec::new("close", ACC_PUBLIC, &[], "V")),
            ClassSpec::new("Ljava/io/BufferedReader;", Some("Ljava/io/Reader;"))
                .with_method(MethodSpec::new(
                    "<init>",
                    ACC_PUBLIC,
                    &["Ljava/io/Reader;"],
                    "V",
                ))
                .with_method(MethodSpec::new(
                    "readLine",
                    ACC_PUBLIC,
                    &[],
                    "Ljava/lang/String;",
                ))
                .with_method(MethodSpec::new("close", ACC_PUBLIC, &[], "V")),
            ClassSpec::new("Ljava/io/FileWriter;", Some("Ljava/io/Writer;"))
                .with_method(MethodSpec::new(
                    "<init>",
                    ACC_PUBLIC,
                    &["Ljava/lang/String;"],
                    "V",
                ))
                .with_method(MethodSpec::new(
                    "write",
                    ACC_PUBLIC,
                    &["Ljava/lang/String;"],
                    "V",
                ))
                .with_method(MethodSpec::new("close", ACC_PUBLIC, &[], "V")),
            ClassSpec::new("LContoh;", Some("Ljava/lang/Object;"))
                .with_method(
                    MethodSpec::new("baca", ACC_PUBLIC | ACC_STATIC, &[], "Ljava/lang/String;")
                        .registers(3)
                        .body(
                            [
                                insn::new_instance(0, buffered),
                                insn::new_instance(1, reader),
                                insn::const_string(2, path),
                                insn::invoke_direct(reader_init, &[1, 2]),
                                insn::invoke_direct(buffered_init, &[0, 1]),
                                insn::invoke_virtual(read_line, &[0]),
                                insn::move_result(0),
                                insn::return_object(0),
                            ]
                            .concat(),
                        )
                        .literal("/data/user/0/com.contoh/files/catatan.txt"),
                )
                .with_method(
                    MethodSpec::new("tulis", ACC_PUBLIC | ACC_STATIC, &[], "V")
                        .registers(3)
                        .body(
                            [
                                insn::new_instance(0, writer),
                                insn::const_string(1, path),
                                insn::invoke_direct(writer_init, &[0, 1]),
                                insn::const_string(2, text),
                                insn::invoke_virtual(write, &[0, 2]),
                                insn::invoke_virtual(close, &[0]),
                                insn::return_void(),
                            ]
                            .concat(),
                        )
                        .literal("halo dunia"),
                ),
        ];
        build_dex_with_symbols(&classes)
    };

    let probe = build(0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0);
    let reader = probe.type_index("Ljava/io/FileReader;");
    let buffered = probe.type_index("Ljava/io/BufferedReader;");
    let writer = probe.type_index("Ljava/io/FileWriter;");
    let reader_init = probe.method_index("Ljava/io/FileReader;", "<init>");
    let buffered_init = probe.method_index("Ljava/io/BufferedReader;", "<init>");
    let read_line = probe.method_index("Ljava/io/BufferedReader;", "readLine");
    let writer_init = probe.method_index("Ljava/io/FileWriter;", "<init>");
    let write = probe.method_index("Ljava/io/FileWriter;", "write");
    let close = probe.method_index("Ljava/io/FileWriter;", "close");
    let path = probe.string_index("/data/user/0/com.contoh/files/catatan.txt");
    let text = probe.string_index("halo dunia");

    let dex_bytes = build(
        reader,
        buffered,
        writer,
        reader_init,
        buffered_init,
        read_line,
        writer_init,
        write,
        close,
        path,
        text,
    )
    .bytes;
    let dex = parse_bytes(&dex_bytes, "teks.dex").expect("DEX fixture harus terparse");
    let mut engine = DexEngine::new(dex);

    let files = std::sync::Arc::new(MemoryFileSystem {
        files: std::sync::Mutex::new(std::collections::HashMap::from([(
            "catatan.txt".to_string(),
            b"baris satu\nbaris dua\n".to_vec(),
        )])),
    });
    engine.set_native_resolver(Box::new(
        AndroidNativeMethods::new().with_file_system(files.clone()),
    ));

    let outcome = engine
        .execute_method("LContoh;", "baca")
        .expect("pembacaan berkas harus berhasil");
    let reference = outcome
        .return_value
        .and_then(Value::as_ref)
        .expect("readLine mengembalikan String");
    assert_eq!(engine.heap.string(reference), Some("baris satu"));

    engine
        .execute_method("LContoh;", "tulis")
        .expect("penulisan berkas harus berhasil");
    let written = files
        .read("catatan.txt")
        .expect("berkas hasil tulis harus ada");
    assert_eq!(written, b"halo dunia".to_vec());
}
