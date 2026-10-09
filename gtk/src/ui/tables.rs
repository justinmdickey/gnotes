//! Tables and kanban boards in notes.

/// A new 3×2 table, as the "/" menu puts it in.
pub const TABLE_TEMPLATE: &str = "| Column 1 | Column 2 | Column 3 |\n| --- | --- | --- |\n|  |  |  |\n|  |  |  |";

/// A new board with To do, Doing and Done, ready for its first card.
pub const BOARD_TEMPLATE: &str = "```kanban\n## To do\n- \n\n## Doing\n\n## Done\n```";
