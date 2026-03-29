/// グリッド座標計算。整数演算のみ（NF1要件）。

/// 1モニター分のグリッド情報
#[derive(Debug, Clone)]
pub struct Grid {
    /// モニター左上の物理ピクセル座標
    pub origin_x: i32,
    pub origin_y: i32,
    /// モニターの幅・高さ（物理ピクセル）
    pub width: i32,
    pub height: i32,
    /// グリッド列数・行数
    pub columns: i32,
    pub rows: i32,
}

impl Grid {
    pub fn new(
        origin_x: i32,
        origin_y: i32,
        width: i32,
        height: i32,
        columns: u32,
        rows: u32,
    ) -> Self {
        Self {
            origin_x,
            origin_y,
            width,
            height,
            columns: columns as i32,
            rows: rows as i32,
        }
    }

    /// 1グリッドセルの幅（切り捨て）
    pub fn cell_width(&self) -> i32 {
        self.width / self.columns
    }

    /// 1グリッドセルの高さ（切り捨て）
    pub fn cell_height(&self) -> i32 {
        self.height / self.rows
    }

    /// 物理ピクセルX座標を最寄りのグリッド列境界（左端）にスナップする。
    /// 戻り値はモニター絶対座標。
    pub fn snap_x(&self, px: i32) -> i32 {
        let relative = px - self.origin_x;
        let cw = self.cell_width();
        let col = (relative as f32 / cw as f32).round() as i32;
        let col = col.clamp(0, self.columns);
        self.origin_x + col * cw
    }

    /// 物理ピクセルY座標を最寄りのグリッド行境界（上端）にスナップする。
    pub fn snap_y(&self, py: i32) -> i32 {
        let relative = py - self.origin_y;
        let ch = self.cell_height();
        let row = (relative as f32 / ch as f32).round() as i32;
        let row = row.clamp(0, self.rows);
        self.origin_y + row * ch
    }

    /// グリッド列インデックス（0-based）を左端X座標（絶対）に変換する。
    pub fn col_to_x(&self, col: i32) -> i32 {
        self.origin_x + col.clamp(0, self.columns) * self.cell_width()
    }

    /// グリッド行インデックス（0-based）を上端Y座標（絶対）に変換する。
    pub fn row_to_y(&self, row: i32) -> i32 {
        self.origin_y + row.clamp(0, self.rows) * self.cell_height()
    }

    /// グリッドセルの絶対座標とサイズを返す（col_span列分、row_span行分）。
    pub fn cell_rect(&self, col: u32, row: u32, col_span: u32, row_span: u32) -> CellRect {
        let x = self.col_to_x(col as i32);
        let y = self.row_to_y(row as i32);
        let w = self.cell_width() * col_span as i32;
        let h = self.cell_height() * row_span as i32;
        CellRect { x, y, w, h }
    }

    /// ピクセル矩形をグリッドセル座標に逆変換する（F0a キャプチャ用）。
    /// 戻り値: (col, row, col_span, row_span)
    pub fn rect_to_cell(&self, x: f64, y: f64, w: f64, h: f64) -> (u32, u32, u32, u32) {
        let cw = self.cell_width() as f64;
        let ch = self.cell_height() as f64;
        if cw <= 0.0 || ch <= 0.0 {
            return (0, 0, 1, 1);
        }
        let rel_x = x - self.origin_x as f64;
        let rel_y = y - self.origin_y as f64;
        let col = (rel_x / cw).round().max(0.0) as u32;
        let row = (rel_y / ch).round().max(0.0) as u32;
        let col_span = (w / cw).round().max(1.0) as u32;
        let row_span = (h / ch).round().max(1.0) as u32;
        // グリッド範囲内にクランプ
        let col = col.min(self.columns as u32 - 1);
        let row = row.min(self.rows as u32 - 1);
        let col_span = col_span.min(self.columns as u32 - col);
        let row_span = row_span.min(self.rows as u32 - row);
        (col, row, col_span, row_span)
    }

    /// すべてのグリッド交点の絶対座標リストを返す（オーバーレイ描画用）。
    pub fn grid_lines(&self) -> GridLines {
        let cw = self.cell_width();
        let ch = self.cell_height();
        let verticals: Vec<i32> = (0..=self.columns)
            .map(|c| self.origin_x + c * cw)
            .collect();
        let horizontals: Vec<i32> = (0..=self.rows)
            .map(|r| self.origin_y + r * ch)
            .collect();
        GridLines {
            verticals,
            horizontals,
            origin_x: self.origin_x,
            origin_y: self.origin_y,
            width: self.width,
            height: self.height,
        }
    }
}

/// スナップ後の矩形
#[derive(Debug, Clone, Copy)]
pub struct CellRect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

/// オーバーレイ描画用グリッド線情報
#[derive(Debug, Clone)]
pub struct GridLines {
    /// 垂直線のX座標リスト（絶対）
    pub verticals: Vec<i32>,
    /// 水平線のY座標リスト（絶対）
    pub horizontals: Vec<i32>,
    /// モニター領域
    pub origin_x: i32,
    pub origin_y: i32,
    pub width: i32,
    pub height: i32,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_grid() -> Grid {
        // 2560x1440, origin (0,0), 8x4
        Grid::new(0, 0, 2560, 1440, 8, 4)
    }

    #[test]
    fn cell_size() {
        let g = make_grid();
        assert_eq!(g.cell_width(), 320);
        assert_eq!(g.cell_height(), 360);
    }

    #[test]
    fn snap_x_midpoint() {
        let g = make_grid();
        // 160px = midpoint of col 0 → snaps to col 1 boundary (320)
        assert_eq!(g.snap_x(160), 320);
        // 319px → still snaps to 320
        assert_eq!(g.snap_x(319), 320);
        // 0px → snaps to 0
        assert_eq!(g.snap_x(0), 0);
    }

    #[test]
    fn snap_clamp() {
        let g = make_grid();
        // 範囲外はモニター端にクランプ
        assert_eq!(g.snap_x(-100), 0);
        assert_eq!(g.snap_x(9999), 2560);
    }

    #[test]
    fn cell_rect_span2() {
        let g = make_grid();
        let r = g.cell_rect(2, 1, 3, 2);
        assert_eq!(r.x, 640);
        assert_eq!(r.y, 360);
        assert_eq!(r.w, 960);
        assert_eq!(r.h, 720);
    }
}