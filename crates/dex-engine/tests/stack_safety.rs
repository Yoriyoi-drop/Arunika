//! Uji ketahanan interpreter terhadap stack host yang kecil (fase 1.6).
//!
//! Rekursi bytecode harus berhenti karena batas kedalaman, bukan karena
//! stack overflow yang mematikan proses. Uji ini menjalankan rekursi tanpa
//! henti di thread dengan stack jauh lebih kecil dari default.

use arunika_dex_engine::{DexEngine, DexError, parse_bytes};
use arunika_test_fixtures::dex::{
    ACC_PUBLIC, ACC_STATIC, ClassSpec, MethodSpec, build_dex_with_symbols, insn,
};

const STACK_BYTES: usize = 512 * 1024;

fn rekursif_dex() -> Vec<u8> {
    let build = |self_index: u16| {
        let method = MethodSpec::new("rekursif", ACC_PUBLIC | ACC_STATIC, &[], "V")
            .registers(0)
            .ins(0)
            .body([insn::invoke_static(self_index, &[]), insn::return_void()].concat());
        build_dex_with_symbols(&[ClassSpec::new("LContoh;", None).with_method(method)])
    };

    let index = build(0).method_index("LContoh;", "rekursif");
    build(index).bytes
}

#[test]
fn rekursi_di_stack_kecil_berhenti_dengan_error() {
    let bytes = rekursif_dex();

    let handle = std::thread::Builder::new()
        .stack_size(STACK_BYTES)
        .spawn(move || {
            let dex = parse_bytes(&bytes, "stack.dex").expect("DEX harus terparse");
            let mut engine = DexEngine::new(dex);
            match engine.execute_method("LContoh;", "rekursif") {
                Err(DexError::CallDepthExceeded { .. }) => true,
                other => {
                    eprintln!("hasil tak terduga: {other:?}");
                    false
                }
            }
        })
        .expect("thread harus dibuat");

    assert!(handle.join().expect("thread tidak boleh abort atau panic"));
}
