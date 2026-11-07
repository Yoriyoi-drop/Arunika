//! Uji parsing DEX hasil pembangkit fixture (bukan hanya header kosong).

use arunika_dex_engine::parse_bytes;
use arunika_test_fixtures::dex::{
    ACC_PUBLIC, ACC_STATIC, CatchSpec, ClassSpec, MethodSpec, build_dex, insn,
};

fn satu_method(insns: Vec<u16>, registers: u16, ins: u16) -> Vec<u8> {
    build_dex(&[
        ClassSpec::new("LContoh;", Some("Ljava/lang/Object;")).with_method(
            MethodSpec::new("jawab", ACC_PUBLIC | ACC_STATIC, &[], "I")
                .registers(registers)
                .ins(ins)
                .body(insns),
        ),
    ])
}

#[test]
fn parser_membaca_instruksi_code_item() {
    let bytes = satu_method([insn::const_16(0, 42), insn::return_(0)].concat(), 1, 0);
    let dex = parse_bytes(&bytes, "uji.dex").expect("DEX fixture harus terparse");

    assert_eq!(dex.method_ids.len(), 1);
    assert_eq!(dex.method_ids[0].name, "jawab");
    assert_eq!(dex.method_ids[0].class, "LContoh;");
    assert_eq!(dex.class_defs.len(), 1);
    assert_eq!(dex.class_defs[0].class, "LContoh;");
    assert_eq!(
        dex.class_defs[0].superclass.as_deref(),
        Some("Ljava/lang/Object;")
    );

    let code = &dex.code_items[0];
    assert_eq!(code.registers_size, 1);
    assert_eq!(code.ins_size, 0);
    assert_eq!(code.outs_size, 0);
    assert_eq!(code.tries_size, 0);
    assert_eq!(code.instructions_size, 3);
    assert_eq!(code.insns, vec![0x13, 42, 0x0f]);
}

#[test]
fn parser_membaca_method_dengan_parameter() {
    let bytes = build_dex(&[ClassSpec::new("LContoh;", None).with_method(
        MethodSpec::new("tambah", ACC_PUBLIC | ACC_STATIC, &["I", "I"], "I")
            .registers(2)
            .ins(2)
            .body([insn::add_int(0, 0, 1), insn::return_(0)].concat()),
    )]);
    let dex = parse_bytes(&bytes, "uji.dex").expect("DEX fixture harus terparse");

    assert_eq!(dex.proto_ids.len(), 1);
    assert_eq!(dex.proto_ids[0].return_type, "I");
    assert_eq!(dex.proto_ids[0].parameter_types, vec!["I", "I"]);

    let code = &dex.code_items[0];
    assert_eq!(code.registers_size, 2);
    assert_eq!(code.ins_size, 2);
    assert_eq!(code.insns, vec![0x90, 0x0100, 0x0f]);
}

#[test]
fn parser_membaca_class_data_direct_dan_virtual() {
    let bytes = build_dex(&[ClassSpec::new("LContoh;", None)
        .with_method(
            MethodSpec::new("statis", ACC_PUBLIC | ACC_STATIC, &[], "V").body(insn::return_void()),
        )
        .with_method(
            MethodSpec::new("virtual", ACC_PUBLIC, &[], "V")
                .registers(1)
                .ins(1)
                .body(insn::return_void()),
        )]);
    let dex = parse_bytes(&bytes, "uji.dex").expect("DEX fixture harus terparse");

    let class_data = dex.class_defs[0]
        .class_data
        .as_ref()
        .expect("class_data harus terbaca");
    assert_eq!(class_data.direct_methods.len(), 1);
    assert_eq!(class_data.virtual_methods.len(), 1);
    assert_eq!(dex.code_items.len(), 2);
}

#[test]
fn parser_membaca_try_dan_catch_handler() {
    let method = MethodSpec::new("jalankan", ACC_PUBLIC | ACC_STATIC, &[], "I")
        .registers(2)
        .ins(0)
        .body(
            [
                insn::new_instance(0, 0),
                insn::throw_(0),
                insn::move_exception(0),
                insn::const_16(1, 7),
                insn::return_(1),
            ]
            .concat(),
        )
        .try_catch(0, 3, CatchSpec::catch_all(3));

    let bytes =
        build_dex(&[ClassSpec::new("LContoh;", Some("Ljava/lang/Object;")).with_method(method)]);
    let dex = parse_bytes(&bytes, "uji.dex").expect("DEX fixture harus terparse");

    let code = &dex.code_items[0];
    assert_eq!(code.tries_size, 1);
    assert_eq!(code.tries[0].start_address, 0);
    assert_eq!(code.tries[0].instruction_count, 3);
    assert_eq!(code.handlers.len(), 1);
    assert_eq!(
        code.handlers[0].offset,
        u32::from(code.tries[0].handler_offset)
    );
    assert!(code.handlers[0].pairs.is_empty());
    assert_eq!(code.handlers[0].catch_all, Some(3));
}

#[test]
fn proto_dua_parameter_terbaca_lengkap() {
    let bytes = build_dex(&[
        ClassSpec::new("LContoh;", Some("Ljava/lang/Object;")).with_method(MethodSpec::new(
            "dua",
            ACC_PUBLIC | ACC_STATIC,
            &["Ljava/lang/String;", "Ljava/lang/String;"],
            "I",
        )),
    ]);

    let dex = parse_bytes(&bytes, "uji.dex").expect("DEX fixture harus terparse");
    let index = dex
        .method_ids
        .iter()
        .position(|method| method.name == "dua")
        .expect("method dua harus ada");
    let proto = &dex.proto_ids[dex.method_ids[index].proto_idx as usize];
    assert_eq!(
        proto.parameter_types,
        vec![
            "Ljava/lang/String;".to_string(),
            "Ljava/lang/String;".to_string()
        ]
    );
}
