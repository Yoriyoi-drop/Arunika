//! Uji opcode tambahan untuk bytecode dunia nyata (fase 1.8).
//!
//! Cakupan: `instance-of`, `check-cast` dengan pemeriksaan tipe,
//! `invoke-interface`, `filled-new-array`, `fill-array-data`, dan
//! `monitor-enter`/`monitor-exit`.

use arunika_dex_engine::{DexEngine, DexError, Value, parse_bytes};
use arunika_test_fixtures::dex::{
    ACC_PUBLIC, ACC_STATIC, ClassSpec, MethodSpec, build_dex_with_symbols, insn,
};

fn engine(bytes: &[u8]) -> DexEngine {
    let dex = parse_bytes(bytes, "opcode.dex").expect("DEX fixture harus terparse");
    DexEngine::new(dex)
}

fn static_method(name: &str, registers: u16, ret: &str, body: Vec<u16>) -> MethodSpec {
    MethodSpec::new(name, ACC_PUBLIC | ACC_STATIC, &[], ret)
        .registers(registers)
        .ins(0)
        .body(body)
}

#[test]
fn instance_of_mengenali_tipe_dan_superclass() {
    let build = |anak: u16, ibu: u16, string: u16| {
        let body = |type_index: u16| {
            [
                insn::new_instance(0, anak),
                insn::instance_of(1, 0, type_index),
                insn::return_(1),
            ]
            .concat()
        };
        build_dex_with_symbols(&[
            ClassSpec::new("LContoh;", Some("Ljava/lang/Object;"))
                .with_method(static_method("cekAnak", 2, "I", body(anak)))
                .with_method(static_method("cekIbu", 2, "I", body(ibu)))
                .with_method(static_method("cekString", 2, "I", body(string))),
            ClassSpec::new("LAnak;", Some("LIbu;")),
            ClassSpec::new("LIbu;", Some("Ljava/lang/Object;")),
            ClassSpec::new("Ljava/lang/String;", Some("Ljava/lang/Object;")),
        ])
    };

    let probe = build(0, 0, 0);
    let bytes = build(
        probe.type_index("LAnak;"),
        probe.type_index("LIbu;"),
        probe.type_index("Ljava/lang/String;"),
    )
    .bytes;

    let mut engine = engine(&bytes);
    let cek = |engine: &mut DexEngine, name: &str| {
        engine
            .execute_method("LContoh;", name)
            .expect("eksekusi harus berhasil")
            .return_value
    };

    assert_eq!(cek(&mut engine, "cekAnak"), Some(Value::Int(1)));
    assert_eq!(cek(&mut engine, "cekIbu"), Some(Value::Int(1)));
    assert_eq!(cek(&mut engine, "cekString"), Some(Value::Int(0)));
}

#[test]
fn check_cast_yang_cocok_dilewatkan() {
    let build = |anak: u16, ibu: u16| {
        build_dex_with_symbols(&[
            ClassSpec::new("LContoh;", Some("Ljava/lang/Object;")).with_method(static_method(
                "jalankan",
                2,
                "I",
                [
                    insn::new_instance(0, anak),
                    insn::check_cast(0, ibu),
                    insn::const_4(1, 3),
                    insn::return_(1),
                ]
                .concat(),
            )),
            ClassSpec::new("LAnak;", Some("LIbu;")),
            ClassSpec::new("LIbu;", Some("Ljava/lang/Object;")),
        ])
    };

    let probe = build(0, 0);
    let bytes = build(probe.type_index("LAnak;"), probe.type_index("LIbu;")).bytes;

    let mut engine = engine(&bytes);
    let outcome = engine
        .execute_method("LContoh;", "jalankan")
        .expect("check-cast yang cocok harus lolos");
    assert_eq!(outcome.return_value, Some(Value::Int(3)));
}

#[test]
fn check_cast_yang_tidak_cocok_melempar_class_cast_exception() {
    let build = |anak: u16, string: u16| {
        build_dex_with_symbols(&[
            ClassSpec::new("LContoh;", Some("Ljava/lang/Object;")).with_method(static_method(
                "jalankan",
                2,
                "I",
                [
                    insn::new_instance(0, anak),
                    insn::check_cast(0, string),
                    insn::const_4(1, 3),
                    insn::return_(1),
                ]
                .concat(),
            )),
            ClassSpec::new("LAnak;", Some("Ljava/lang/Object;")),
            ClassSpec::new("Ljava/lang/String;", Some("Ljava/lang/Object;")),
        ])
    };

    let probe = build(0, 0);
    let bytes = build(
        probe.type_index("LAnak;"),
        probe.type_index("Ljava/lang/String;"),
    )
    .bytes;

    let mut engine = engine(&bytes);
    let error = engine
        .execute_method("LContoh;", "jalankan")
        .expect_err("check-cast yang salah harus melempar");
    match error {
        DexError::UncaughtException { type_name } => {
            assert_eq!(type_name, "Ljava/lang/ClassCastException;");
        }
        other => panic!("error tak terduga: {other:?}"),
    }
}

#[test]
fn invoke_interface_memanggil_method_yang_ada() {
    let build = |nilai: u16| {
        let impl_method = MethodSpec::new("nilai", ACC_PUBLIC, &[], "I")
            .registers(1)
            .ins(1)
            .body([insn::const_16(0, 7), insn::return_(0)].concat());
        let caller = static_method(
            "jalankan",
            2,
            "I",
            [
                insn::new_instance(0, 0),
                insn::invoke_interface(nilai, &[0]),
                insn::move_result(1),
                insn::return_(1),
            ]
            .concat(),
        );
        build_dex_with_symbols(&[
            ClassSpec::new("LImpl;", Some("Ljava/lang/Object;")).with_method(impl_method),
            ClassSpec::new("LContoh;", Some("Ljava/lang/Object;")).with_method(caller),
        ])
    };

    let probe = build(0);
    let nilai = probe.method_index("LImpl;", "nilai");
    let mut engine = engine(&build(nilai).bytes);
    let outcome = engine
        .execute_method("LContoh;", "jalankan")
        .expect("invoke-interface harus berhasil");
    assert_eq!(outcome.return_value, Some(Value::Int(7)));
}

#[test]
fn filled_new_array_membuat_array_dari_register() {
    let build = |int_type: u16| {
        build_dex_with_symbols(&[ClassSpec::new("LContoh;", Some("Ljava/lang/Object;"))
            .with_method(static_method(
                "jalankan",
                6,
                "I",
                [
                    insn::const_4(0, 5),
                    insn::const_4(1, 6),
                    insn::const_4(2, 7),
                    insn::filled_new_array(int_type, &[0, 1, 2]),
                    insn::move_result(3),
                    insn::const_4(4, 2),
                    insn::aget(5, 3, 4),
                    insn::return_(5),
                ]
                .concat(),
            ))])
    };

    let int_type = build(0).type_index("I");
    let mut engine = engine(&build(int_type).bytes);
    let outcome = engine
        .execute_method("LContoh;", "jalankan")
        .expect("filled-new-array harus berhasil");
    assert_eq!(outcome.return_value, Some(Value::Int(7)));
}

#[test]
fn fill_array_data_mengisi_array_dari_payload() {
    let build = |int_type: u16| {
        let mut body = [
            insn::const_4(0, 3),
            insn::new_array(0, 0, int_type),
            insn::fill_array_data(0, 7),
            insn::const_4(1, 1),
            insn::aget(2, 0, 1),
            insn::return_(2),
        ]
        .concat();
        body.extend(insn::array_data_i32(&[10, 20, 30]));

        build_dex_with_symbols(&[ClassSpec::new("LContoh;", Some("Ljava/lang/Object;"))
            .with_method(static_method("jalankan", 3, "I", body))])
    };

    let int_type = build(0).type_index("I");
    let mut engine = engine(&build(int_type).bytes);
    let outcome = engine
        .execute_method("LContoh;", "jalankan")
        .expect("fill-array-data harus berhasil");
    assert_eq!(outcome.return_value, Some(Value::Int(20)));
}

#[test]
fn fill_array_data_dengan_panjang_berbeda_ditolak() {
    let build = |int_type: u16| {
        let mut body = [
            insn::const_4(0, 2),
            insn::new_array(0, 0, int_type),
            insn::fill_array_data(0, 5),
            insn::const_4(1, 0),
            insn::return_(1),
        ]
        .concat();
        body.extend(insn::array_data_i32(&[10, 20, 30]));

        build_dex_with_symbols(&[ClassSpec::new("LContoh;", Some("Ljava/lang/Object;"))
            .with_method(static_method("jalankan", 2, "I", body))])
    };

    let int_type = build(0).type_index("I");
    let mut engine = engine(&build(int_type).bytes);
    let error = engine
        .execute_method("LContoh;", "jalankan")
        .expect_err("panjang payload yang berbeda harus ditolak");
    match error {
        DexError::ArrayDataMismatch { payload, array } => {
            assert_eq!(payload, 3);
            assert_eq!(array, 2);
        }
        other => panic!("error tak terduga: {other:?}"),
    }
}

#[test]
fn monitor_enter_dan_exit_tidak_mengubah_hasil() {
    let build = |tipe: u16| {
        build_dex_with_symbols(&[
            ClassSpec::new("LContoh;", Some("Ljava/lang/Object;")).with_method(static_method(
                "jalankan",
                2,
                "I",
                [
                    insn::new_instance(0, tipe),
                    insn::monitor_enter(0),
                    insn::monitor_exit(0),
                    insn::const_4(1, 3),
                    insn::return_(1),
                ]
                .concat(),
            )),
            ClassSpec::new("LObj;", Some("Ljava/lang/Object;")),
        ])
    };

    let tipe = build(0).type_index("LObj;");
    let mut engine = engine(&build(tipe).bytes);
    let outcome = engine
        .execute_method("LContoh;", "jalankan")
        .expect("monitor harus diabaikan");
    assert_eq!(outcome.return_value, Some(Value::Int(3)));
}

#[test]
fn const_class_mengembalikan_referensi_yang_sama_untuk_tipe_sama() {
    let build = |type_index: u16| {
        let body = [
            insn::const_class(0, type_index),
            insn::const_class(1, type_index),
            insn::if_eq(0, 1, 6),
            insn::const_4(2, 0),
            insn::return_(2),
            insn::nop(),
            insn::nop(),
            insn::const_4(2, 1),
            insn::return_(2),
        ]
        .concat();
        build_dex_with_symbols(&[
            ClassSpec::new("LContoh;", None).with_method(static_method("jalankan", 3, "I", body))
        ])
    };

    let type_index = build(0).type_index("LContoh;");
    let mut engine = engine(&build(type_index).bytes);
    let outcome = engine
        .execute_method("LContoh;", "jalankan")
        .expect("const-class harus berhasil");

    assert_eq!(outcome.return_value, Some(Value::Int(1)));
}

#[test]
fn if_eqz_pada_referensi_tidak_dianggap_null() {
    let build = |type_index: u16| {
        let body = [
            insn::const_class(0, type_index),
            insn::if_eqz(0, 4),
            insn::const_4(1, 1),
            insn::return_(1),
            insn::const_4(1, 0),
            insn::return_(1),
        ]
        .concat();
        build_dex_with_symbols(&[
            ClassSpec::new("LContoh;", None).with_method(static_method("jalankan", 2, "I", body))
        ])
    };

    let type_index = build(0).type_index("LContoh;");
    let mut engine = engine(&build(type_index).bytes);
    let outcome = engine
        .execute_method("LContoh;", "jalankan")
        .expect("if-eqz harus berhasil");

    assert_eq!(outcome.return_value, Some(Value::Int(1)));
}
#[test]
fn const_string_jumbo_membaca_indeks_32_bit() {
    let build = |index: u32| {
        let method = static_method(
            "jumbo",
            1,
            "Ljava/lang/String;",
            [insn::const_string_jumbo(0, index), insn::return_object(0)].concat(),
        )
        .literal("teks-jumbo");
        build_dex_with_symbols(&[ClassSpec::new("LContoh;", None).with_method(method)])
    };

    let probe = build(0);
    let index = u32::from(probe.string_index("teks-jumbo"));
    let bytes = build(index).bytes;

    let mut engine = engine(&bytes);
    let outcome = engine
        .execute_method("LContoh;", "jumbo")
        .expect("eksekusi harus berhasil");
    let reference = outcome
        .return_value
        .and_then(Value::as_ref)
        .expect("const-string/jumbo harus mengembalikan String");
    assert_eq!(engine.heap.string(reference), Some("teks-jumbo"));
}
#[test]
fn binop_dua_alamat_memakai_register_tujuan_sebagai_operand() {
    let int_method = static_method(
        "tambahInt",
        2,
        "I",
        [
            insn::const_16(0, 40),
            insn::const_16(1, 2),
            insn::binop_2addr(0xb0, 0, 1),
            insn::return_(0),
        ]
        .concat(),
    );
    let long_method = static_method(
        "tambahLong",
        4,
        "J",
        [
            insn::const_wide_16(0, 100),
            insn::const_wide_16(2, 23),
            insn::binop_2addr(0xbb, 0, 2),
            insn::return_wide(0),
        ]
        .concat(),
    );

    let bytes = build_dex_with_symbols(&[ClassSpec::new("LContoh;", None)
        .with_method(int_method)
        .with_method(long_method)])
    .bytes;

    let mut engine = engine(&bytes);
    let int = engine
        .execute_method("LContoh;", "tambahInt")
        .expect("add-int/2addr harus berhasil");
    assert_eq!(int.return_value, Some(Value::Int(42)));

    let long = engine
        .execute_method("LContoh;", "tambahLong")
        .expect("add-long/2addr harus berhasil");
    assert_eq!(long.return_value, Some(Value::from_long(123)));
}
#[test]
fn aritmetika_dengan_literal_kecil_dan_menengah_benar() {
    // v0 = 10; += 5; *= 3; += 1000; lalu rsub 20 (20 - v0).
    let body = [
        insn::const_16(0, 10),
        insn::binop_lit8(0xd8, 0, 0, 5),
        insn::binop_lit8(0xda, 0, 0, 3),
        insn::binop_lit16(0xd0, 0, 0, 1000),
        insn::binop_lit8(0xd9, 0, 0, 20),
        insn::return_(0),
    ]
    .concat();

    let bytes = build_dex_with_symbols(&[ClassSpec::new("LContoh;", Some("Ljava/lang/Object;"))
        .with_method(static_method("hitung", 1, "I", body))])
    .bytes;
    let mut engine = engine(&bytes);
    let outcome = engine
        .execute_method("LContoh;", "hitung")
        .expect("aritmetika literal harus berhasil");
    assert_eq!(outcome.return_value, Some(Value::Int(-1025)));
}

#[test]
fn ushr_literal_memperlakukan_nilai_sebagai_tanpa_tanda() {
    let body = [
        insn::const_16(0, -1),
        insn::binop_lit8(0xe2, 0, 0, 1),
        insn::return_(0),
    ]
    .concat();
    let bytes = build_dex_with_symbols(&[ClassSpec::new("LContoh;", Some("Ljava/lang/Object;"))
        .with_method(static_method("geser", 1, "I", body))])
    .bytes;
    let mut engine = engine(&bytes);
    let outcome = engine
        .execute_method("LContoh;", "geser")
        .expect("ushr literal harus berhasil");
    assert_eq!(outcome.return_value, Some(Value::Int(i32::MAX)));
}

#[test]
fn packed_switch_memilih_cabang_berdasarkan_kunci() {
    let body = |first_key: i32, targets: &[i32]| {
        let mut units = [
            insn::packed_switch(1, 6),
            insn::const_4(0, 0),
            insn::return_(0),
            insn::nop(),
        ]
        .concat();
        units.extend(insn::packed_switch_payload(first_key, targets));
        units.extend([insn::const_16(0, 10), insn::return_(0)].concat());
        units.extend([insn::const_16(0, 20), insn::return_(0)].concat());
        units
    };

    let method = MethodSpec::new("pilih", ACC_PUBLIC | ACC_STATIC, &["I"], "I")
        .registers(2)
        .ins(1)
        .body(body(1, &[14, 17]));
    let bytes = build_dex_with_symbols(&[
        ClassSpec::new("LContoh;", Some("Ljava/lang/Object;")).with_method(method)
    ])
    .bytes;
    let mut engine = engine(&bytes);

    let pilih = |engine: &mut DexEngine, nilai: i32| {
        engine
            .execute_method_values("LContoh;", "pilih", &[Value::Int(nilai)])
            .expect("switch harus berhasil")
            .return_value
    };
    assert_eq!(pilih(&mut engine, 1), Some(Value::Int(10)));
    assert_eq!(pilih(&mut engine, 2), Some(Value::Int(20)));
    assert_eq!(
        pilih(&mut engine, 0),
        Some(Value::Int(0)),
        "di luar rentang"
    );
    assert_eq!(
        pilih(&mut engine, 3),
        Some(Value::Int(0)),
        "di luar rentang"
    );
}

#[test]
fn sparse_switch_memilih_cabang_dari_daftar_kunci() {
    let mut units = [
        insn::sparse_switch(1, 6),
        insn::const_4(0, 0),
        insn::return_(0),
        insn::nop(),
    ]
    .concat();
    units.extend(insn::sparse_switch_payload(&[5, 100], &[16, 19]));
    units.extend([insn::const_16(0, 10), insn::return_(0)].concat());
    units.extend([insn::const_16(0, 20), insn::return_(0)].concat());

    let method = MethodSpec::new("pilih", ACC_PUBLIC | ACC_STATIC, &["I"], "I")
        .registers(2)
        .ins(1)
        .body(units);
    let bytes = build_dex_with_symbols(&[
        ClassSpec::new("LContoh;", Some("Ljava/lang/Object;")).with_method(method)
    ])
    .bytes;
    let mut engine = engine(&bytes);

    let pilih = |engine: &mut DexEngine, nilai: i32| {
        engine
            .execute_method_values("LContoh;", "pilih", &[Value::Int(nilai)])
            .expect("switch harus berhasil")
            .return_value
    };
    assert_eq!(pilih(&mut engine, 5), Some(Value::Int(10)));
    assert_eq!(pilih(&mut engine, 100), Some(Value::Int(20)));
    assert_eq!(
        pilih(&mut engine, 7),
        Some(Value::Int(0)),
        "kunci tidak ada"
    );
}
