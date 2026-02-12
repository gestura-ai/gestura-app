//! Animated "thinking" indicator for the composer title bar.
//!
//! Uses the dots8bit braille pattern (256 frames) with the Gestura brand gradient
//! (blue-400 → purple-400) and a rotating set of thinking words that cycle with
//! a smooth character-by-character roll animation.

use ratatui::{
    style::{Color, Modifier, Style},
    text::Span,
};

/// All 256 braille frames from the dots8bit spinner pattern.
const DOTS8BIT: [&str; 256] = [
    "⠀", "⠁", "⠂", "⠃", "⠄", "⠅", "⠆", "⠇", "⡀", "⡁", "⡂", "⡃", "⡄", "⡅", "⡆", "⡇", "⠈", "⠉", "⠊",
    "⠋", "⠌", "⠍", "⠎", "⠏", "⡈", "⡉", "⡊", "⡋", "⡌", "⡍", "⡎", "⡏", "⠐", "⠑", "⠒", "⠓", "⠔", "⠕",
    "⠖", "⠗", "⡐", "⡑", "⡒", "⡓", "⡔", "⡕", "⡖", "⡗", "⠘", "⠙", "⠚", "⠛", "⠜", "⠝", "⠞", "⠟", "⡘",
    "⡙", "⡚", "⡛", "⡜", "⡝", "⡞", "⡟", "⠠", "⠡", "⠢", "⠣", "⠤", "⠥", "⠦", "⠧", "⡠", "⡡", "⡢", "⡣",
    "⡤", "⡥", "⡦", "⡧", "⠨", "⠩", "⠪", "⠫", "⠬", "⠭", "⠮", "⠯", "⡨", "⡩", "⡪", "⡫", "⡬", "⡭", "⡮",
    "⡯", "⠰", "⠱", "⠲", "⠳", "⠴", "⠵", "⠶", "⠷", "⡰", "⡱", "⡲", "⡳", "⡴", "⡵", "⡶", "⡷", "⠸", "⠹",
    "⠺", "⠻", "⠼", "⠽", "⠾", "⠿", "⡸", "⡹", "⡺", "⡻", "⡼", "⡽", "⡾", "⡿", "⢀", "⢁", "⢂", "⢃", "⢄",
    "⢅", "⢆", "⢇", "⣀", "⣁", "⣂", "⣃", "⣄", "⣅", "⣆", "⣇", "⢈", "⢉", "⢊", "⢋", "⢌", "⢍", "⢎", "⢏",
    "⣈", "⣉", "⣊", "⣋", "⣌", "⣍", "⣎", "⣏", "⢐", "⢑", "⢒", "⢓", "⢔", "⢕", "⢖", "⢗", "⣐", "⣑", "⣒",
    "⣓", "⣔", "⣕", "⣖", "⣗", "⢘", "⢙", "⢚", "⢛", "⢜", "⢝", "⢞", "⢟", "⣘", "⣙", "⣚", "⣛", "⣜", "⣝",
    "⣞", "⣟", "⢠", "⢡", "⢢", "⢣", "⢤", "⢥", "⢦", "⢧", "⣠", "⣡", "⣢", "⣣", "⣤", "⣥", "⣦", "⣧", "⢨",
    "⢩", "⢪", "⢫", "⢬", "⢭", "⢮", "⢯", "⣨", "⣩", "⣪", "⣫", "⣬", "⣭", "⣮", "⣯", "⢰", "⢱", "⢲", "⢳",
    "⢴", "⢵", "⢶", "⢷", "⣰", "⣱", "⣲", "⣳", "⣴", "⣵", "⣶", "⣷", "⢸", "⢹", "⢺", "⢻", "⢼", "⢽", "⢾",
    "⢿", "⣸", "⣹", "⣺", "⣻", "⣼", "⣽", "⣾", "⣿",
];

/// Thinking words that rotate with a roll animation.
const THINKING_WORDS: [&str; 8] = [
    "Thinking",
    "Pondering",
    "Contemplating",
    "Reflecting",
    "Reasoning",
    "Analyzing",
    "Processing",
    "Considering",
];

/// Brand gradient endpoints.
const GRADIENT_START: (u8, u8, u8) = (96, 165, 250); // blue-400
const GRADIENT_END: (u8, u8, u8) = (192, 132, 252); // purple-400

/// Ticks per word before rolling to the next (at ~50ms per tick ≈ 2.5s per word).
const TICKS_PER_WORD: u64 = 50;

/// Number of ticks for the roll transition animation.
const ROLL_TICKS: u64 = 12;

/// Linearly interpolate between two RGB colors at position `t` ∈ [0.0, 1.0].
fn lerp_rgb(t: f64) -> (u8, u8, u8) {
    let r = (GRADIENT_START.0 as f64 + (GRADIENT_END.0 as f64 - GRADIENT_START.0 as f64) * t)
        .round() as u8;
    let g = (GRADIENT_START.1 as f64 + (GRADIENT_END.1 as f64 - GRADIENT_START.1 as f64) * t)
        .round() as u8;
    let b = (GRADIENT_START.2 as f64 + (GRADIENT_END.2 as f64 - GRADIENT_START.2 as f64) * t)
        .round() as u8;
    (r, g, b)
}

/// Compute the gradient color for the current tick using a ping-pong sweep.
fn gradient_color(tick: u64) -> Color {
    // Sweep over 256 frames (one full dots8bit cycle), ping-pong.
    let pos = (tick % 512) as f64 / 256.0; // 0..2
    let t = if pos <= 1.0 { pos } else { 2.0 - pos };
    let (r, g, b) = lerp_rgb(t);
    Color::Rgb(r, g, b)
}

/// Build the animated thinking title as a list of styled spans for the composer.
///
/// Returns spans like: `" ⣻ Thinking "` with the braille char gradient-colored
/// and the word smoothly rolling between thinking synonyms.
pub(crate) fn thinking_title_spans(tick: u64) -> Vec<Span<'static>> {
    let frame_idx = (tick as usize) % DOTS8BIT.len();
    let braille = DOTS8BIT[frame_idx];
    let color = gradient_color(tick);

    // Determine current and next word.
    let word_cycle = tick / TICKS_PER_WORD;
    let tick_in_word = tick % TICKS_PER_WORD;
    let current_idx = (word_cycle as usize) % THINKING_WORDS.len();
    let next_idx = (current_idx + 1) % THINKING_WORDS.len();

    let current_word = THINKING_WORDS[current_idx];
    let next_word = THINKING_WORDS[next_idx];

    // During the last ROLL_TICKS of a word, do a character-by-character roll.
    let display_word = if tick_in_word >= TICKS_PER_WORD - ROLL_TICKS {
        let roll_progress = tick_in_word - (TICKS_PER_WORD - ROLL_TICKS);
        let max_len = current_word.len().max(next_word.len());
        // How many characters have "rolled" to the next word.
        let chars_rolled = if max_len == 0 {
            0
        } else {
            ((roll_progress + 1) as usize * max_len) / (ROLL_TICKS as usize)
        };

        // Build the transitioning string: first `chars_rolled` chars from next word,
        // remainder from current word.
        let mut result = String::with_capacity(max_len);
        for pos in 0..max_len {
            if pos < chars_rolled {
                result.push(next_word.chars().nth(pos).unwrap_or(' '));
            } else {
                result.push(current_word.chars().nth(pos).unwrap_or(' '));
            }
        }
        result.trim_end().to_string()
    } else {
        current_word.to_string()
    };

    vec![
        Span::raw(" "),
        Span::styled(
            braille.to_string(),
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!(" {} ", display_word),
            Style::default().fg(color).add_modifier(Modifier::DIM),
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thinking_title_returns_three_spans() {
        let spans = thinking_title_spans(0);
        assert_eq!(spans.len(), 3, "expected leading space + braille + word");
    }

    #[test]
    fn braille_frame_cycles_through_all_256() {
        // Frame 0 should be the empty braille, frame 255 should be full.
        let first = thinking_title_spans(0);
        let last = thinking_title_spans(255);
        assert!(first[1].content.contains('⠀'), "first frame should be ⠀");
        assert!(last[1].content.contains('⣿'), "frame 255 should be ⣿");
    }

    #[test]
    fn word_rotates_after_ticks_per_word() {
        // At tick 0, word should be "Thinking".
        let spans_0 = thinking_title_spans(0);
        assert!(
            spans_0[2].content.contains("Thinking"),
            "tick 0 word: {:?}",
            spans_0[2].content
        );

        // Well into the second word cycle, should show "Pondering".
        let tick = TICKS_PER_WORD + 1;
        let spans_1 = thinking_title_spans(tick);
        assert!(
            spans_1[2].content.contains("Pondering"),
            "tick {} word: {:?}",
            tick,
            spans_1[2].content
        );
    }

    #[test]
    fn gradient_color_endpoints() {
        // At tick 0, color should be the gradient start (blue).
        let c = gradient_color(0);
        assert_eq!(c, Color::Rgb(96, 165, 250));

        // At tick 256, color should be the gradient end (purple).
        let c = gradient_color(256);
        assert_eq!(c, Color::Rgb(192, 132, 252));
    }

    #[test]
    fn roll_animation_produces_valid_strings() {
        // During the roll transition (last ROLL_TICKS of a word), the display
        // word should be non-empty and not contain only spaces.
        let roll_start = TICKS_PER_WORD - ROLL_TICKS;
        for t in roll_start..TICKS_PER_WORD {
            let spans = thinking_title_spans(t);
            let word_content = spans[2].content.trim().to_string();
            assert!(
                !word_content.is_empty(),
                "roll tick {t} produced empty word"
            );
        }
    }
}
