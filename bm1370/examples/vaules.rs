use bm1370::BM1370;

fn main() {
    let mut bm1370 = BM1370::default();
    bm1370.plls[BM1370_PLL_ID_UART].set_parameter(0x5aa5_5aa5);
    bm1370.plls[BM1370_PLL_ID_UART]
        .set_parameter(0x5aa5_5aa5) // TODO: replace these fixed values with equivalent individual ones below
        .lock()
        .enable()
        .set_fb_div(112)
        .set_ref_div(1)
        .set_post1_div(1)
        .set_post2_div(1)
        .set_out_div(BM1370_PLL_OUT_UART, pll3_div4);
    let pll3_param = bm1370.plls[BM1370_PLL_ID_UART].parameter();
    let pll3_param = 0x5aa5_5aa5;
}
