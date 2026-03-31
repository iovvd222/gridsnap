/// GridSnap 統合テスト
///
/// Config → Grid → snap のE2Eフローを検証する。
/// tests/ ディレクトリなので公開APIのみアクセス可能（use gridsnap::*）。

use gridsnap::config::*;
use gridsnap::grid::Grid;

// ================================================================
// Config → Grid → snap のE2Eフロー
// ================================================================

#[test]
fn config_to_grid_to_snap_pipeline() {
    let mut config = Config::default();
    config.grid.columns = 8;
    config.grid.rows = 4;

    let gc = config.grid_for_monitor("DISPLAY1");
    let g = Grid::new(0, 0, 3840, 2160, gc.columns, gc.rows);

    // cell_width = 3840/8 = 480, cell_height = 2160/4 = 540
    assert_eq!(g.cell_width(), 480);
    assert_eq!(g.cell_height(), 540);

    // ウィンドウ左上 (500, 300) をスナップ
    let sx = g.snap_x(500);
    let sy = g.snap_y(300);
    assert_eq!(sx, 480); // |500-480|=20 < |500-960|=460
    // |300-0|=300 > |300-540|=240 → 540 が近い
    assert_eq!(sy, 540);
}

#[test]
fn app_rule_to_cell_rect() {
    let config = Config::default(); // 20x12
    let g = Grid::new(0, 0, 3840, 2160, config.grid.columns, config.grid.rows);

    let rule = AppRule {
        monitor: None,
        class_name: None,
        exe_name: Some("Code.exe".into()),
        col: 0, row: 0, col_span: 10, row_span: 12,
    };

    let r = g.cell_rect(rule.col, rule.row, rule.col_span, rule.row_span);
    // 20列のうち10列 = 半分
    assert_eq!(r.x, 0);
    assert_eq!(r.w, 1920); // 3840 / 2
    assert_eq!(r.h, 2160); // 全行
}

#[test]
fn capture_and_restore_roundtrip() {
    // F0a: ウィンドウ位置キャプチャ → ルール保存 → F0b: ルール適用で同じ位置に戻る
    let config = Config::default();
    let g = Grid::new(0, 0, 2560, 1440, config.grid.columns, config.grid.rows);

    // 仮想ウィンドウ位置: (384, 0) size (640, 720)
    let (col, row, cs, rs) = g.rect_to_cell(384.0, 0.0, 640.0, 720.0);

    // ルールから復元
    let restored = g.cell_rect(col, row, cs, rs);
    // 復元した矩形が元のウィンドウ位置を包含する
    assert!(restored.x <= 384);
    assert!(restored.y <= 0);
    assert!(restored.x + restored.w >= 384 + 640 - 20); // 端数許容
}

// ================================================================
// マルチモニターシナリオ
// ================================================================

#[test]
fn multimonitor_independent_grids() {
    let mut config = Config::default();
    config.monitor_grids.insert(
        "DISPLAY2".into(),
        MonitorGridConfig { columns: Some(10), rows: Some(6) },
    );

    let gc1 = config.grid_for_monitor("DISPLAY1");
    let gc2 = config.grid_for_monitor("DISPLAY2");

    let g1 = Grid::new(0, 0, 3840, 2160, gc1.columns, gc1.rows);
    let g2 = Grid::new(3840, 0, 1920, 1080, gc2.columns, gc2.rows);

    // モニター1: 20x12
    assert_eq!(g1.cell_width(), 192);
    // モニター2: 10x6
    assert_eq!(g2.cell_width(), 192);
    // 座標空間が重ならない
    assert_eq!(g1.col_to_x(20), g2.col_to_x(0));
}

// ================================================================
// エッジケース: UWQHD / 4K
// ================================================================

#[test]
fn uwqhd_grid_coverage() {
    let g = Grid::new(0, 0, 3440, 1440, 20, 12);
    assert_eq!(g.cell_width(), 172);
    let r = g.cell_rect(0, 0, 20, 12);
    assert_eq!(r.w, 3440);
    assert_eq!(r.h, 1440);
}

#[test]
fn uhd_4k_no_remainder() {
    let g = Grid::new(0, 0, 3840, 2160, 20, 12);
    assert_eq!(g.cell_width(), 192);
    assert_eq!(g.cell_height(), 180);
    assert_eq!(g.col_to_x(20), 3840);
    assert_eq!(g.row_to_y(12), 2160);
}