//! Uji inisialisasi class otomatis: clinit berjalan sekali saat class dipakai.

use arunika_dex_engine::{DexEngine, Value, parse_bytes};
use arunika_test_fixtures::dex::{
    ACC_PUBLIC, ACC_STATIC, ClassSpec, FieldSpec, MethodSpec, build_dex_with_symbols, insn,
};

fn build(siap: u16, jumlah: u16) -> Vec<u8> {
    let kelas = ClassSpec::new("LKelas;", Some("Ljava/lang/Object;"))
        .with_field(FieldSpec::new("siap", "I", true))
        .with_field(FieldSpec::new("jumlah", "I", true))
        .with_method(
            MethodSpec::new("<clinit>", ACC_STATIC, &[], "V")
                .registers(2)
                .body(
                    [
                        insn::const_16(0, 7),
                        insn::sput(0, siap),
                        insn::sget(0, jumlah),
                        insn::const_16(1, 1),
                        insn::add_int(0, 0, 1),
                        insn::sput(0, jumlah),
                        insn::return_void(),
                    ]
                    .concat(),
                ),
        )
        .with_method(
            MethodSpec::new("ambil", ACC_PUBLIC | ACC_STATIC, &[], "I")
                .registers(1)
                .body([insn::sget(0, siap), insn::return_(0)].concat()),
        );
    build_dex_with_symbols(&[kelas]).bytes
}

#[test]
fn clinit_berjalan_sekali_saat_class_dipakai() {
    let probe = build(0, 0);
    let dex = parse_bytes(&probe, "clinit.dex").expect("DEX fixture harus terparse");
    let siap = dex
        .field_ids
        .iter()
        .position(|field| field.name == "siap")
        .expect("field siap ada") as u16;
    let jumlah = dex
        .field_ids
        .iter()
        .position(|field| field.name == "jumlah")
        .expect("field jumlah ada") as u16;

    let bytes = build(siap, jumlah);
    let dex = parse_bytes(&bytes, "clinit.dex").expect("DEX fixture harus terparse");
    let mut engine = DexEngine::new(dex);

    let first = engine
        .execute_method("LKelas;", "ambil")
        .expect("pemanggilan pertama harus berhasil");
    assert_eq!(first.return_value, Some(Value::Int(7)));
    let second = engine
        .execute_method("LKelas;", "ambil")
        .expect("pemanggilan kedua harus berhasil");
    assert_eq!(second.return_value, Some(Value::Int(7)));

    assert_eq!(engine.statics.get(&(jumlah as u32)), Some(&Value::Int(1)));
    assert!(engine.class_state.initialized.contains("LKelas;"));
}
