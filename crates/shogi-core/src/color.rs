/// Side to move
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Color {
    Black = 0, // Sente (first player, moves up)
    White = 1, // Gote  (second player, moves down)
}

impl Color {
    #[inline]
    pub const fn flip(self) -> Self {
        match self {
            Color::Black => Color::White,
            Color::White => Color::Black,
        }
    }

    #[inline]
    pub const fn index(self) -> usize {
        self as usize
    }
}
