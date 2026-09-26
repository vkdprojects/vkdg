//! Output style injector: appends a style directive to the system prompt.
//!
//! Output styles steer model verbosity without compression.
//! They add a system prompt suffix requesting a specific format.

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutputStyle {
    /// Short paragraphs, minimal hedging, direct answers.
    TerseProse,
    /// Fewer code examples; prefer description over implementation.
    LessCode,
    /// The "ponytail" developer style: extremely terse, fragments, no ceremony.
    Ponytail,
    /// ADHD-optimized: bullet points, headers, scannable structure.
    Adhd,
    /// Compact CJK-aware: short lines, avoid padding characters.
    TerseCjk,
}

impl OutputStyle {
    pub fn system_suffix(&self) -> &'static str {
        match self {
            Self::TerseProse => "\n\nRespond concisely. No preamble, no hedging, no closing summary. Paragraphs only.",
            Self::LessCode => "\n\nMinimize code examples. Prefer description over implementation unless code is the only clear explanation.",
            Self::Ponytail => "\n\nExtremely terse. Fragments OK. No ceremony. Fastest path to the answer.",
            Self::Adhd => "\n\nUse bullet points and short headers. No walls of text. Scannable structure. Bold the key term per paragraph.",
            Self::TerseCjk => "\n\n简洁回答。避免填充词和重复。直接给出答案。",
        }
    }
}
