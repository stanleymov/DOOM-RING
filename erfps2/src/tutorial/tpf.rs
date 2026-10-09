use std::sync::Once;

use eldenring::cs::{TpfRepository, TpfResCap};
use windows::core::{PCWSTR, w};

use crate::{program::Program, rva::LOAD_TPF_RES_CAP_RVA};

static TUTORIAL_TPF: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/MENU_Tuto_18949.tpf"));

pub fn load_tutorial_tpf(tpf_repo: &mut TpfRepository) {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| unsafe {
        let load_tpf_res_cap = Program::current().derva_ptr::<unsafe extern "C" fn(
            *mut TpfRepository,
            PCWSTR,
            *const u8,
            usize,
            bool,
            u32,
        ) -> *mut TpfResCap>(LOAD_TPF_RES_CAP_RVA);

        load_tpf_res_cap(
            tpf_repo,
            w!("MENU_Tuto_18949"),
            TUTORIAL_TPF.as_ptr(),
            TUTORIAL_TPF.len(),
            false,
            0,
        );
    });
}
