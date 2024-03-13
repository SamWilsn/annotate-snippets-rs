//! display_list module stores the output model for the snippet.
//!
//! `DisplayList` is a central structure in the crate, which contains
//! the structured list of lines to be displayed.
//!
//! It is made of two types of lines: `Source` and `Raw`. All `Source` lines
//! are structured using four columns:
//!
//! ```text
//!  /------------ (1) Line number column.
//!  |  /--------- (2) Line number column delimiter.
//!  |  | /------- (3) Inline marks column.
//!  |  | |   /--- (4) Content column with the source and annotations for slices.
//!  |  | |   |
//! =============================================================================
//! error[E0308]: mismatched types
//!    --> src/format.rs:51:5
//!     |
//! 151 | /   fn test() -> String {
//! 152 | |       return "test";
//! 153 | |   }
//!     | |___^ error: expected `String`, for `&str`.
//!     |
//! ```
//!
//! The first two lines of the example above are `Raw` lines, while the rest
//! are `Source` lines.
//!
//! `DisplayList` does not store column alignment information, and those are
//! only calculated by the implementation of `std::fmt::Display` using information such as
//! styling.
//!
//! The above snippet has been built out of the following structure:
use crate::snippet;
use std::fmt::{Display, Write};
use std::ops::Range;
use std::{cmp, fmt};

use crate::renderer::{stylesheet::Stylesheet, Margin, Style};

const ANONYMIZED_LINE_NUM: &str = "LL";
const ERROR_TXT: &str = "error";
const HELP_TXT: &str = "help";
const INFO_TXT: &str = "info";
const NOTE_TXT: &str = "note";
const WARNING_TXT: &str = "warning";

/// List of lines to be displayed.
pub(crate) struct DisplayList<'a> {
    pub body: Vec<DisplaySet<'a>>,
    pub stylesheet: &'a Stylesheet,
    pub anonymized_line_numbers: bool,
}

impl<'a> PartialEq for DisplayList<'a> {
    fn eq(&self, other: &Self) -> bool {
        self.body == other.body && self.anonymized_line_numbers == other.anonymized_line_numbers
    }
}

impl<'a> fmt::Debug for DisplayList<'a> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DisplayList")
            .field("body", &self.body)
            .field("anonymized_line_numbers", &self.anonymized_line_numbers)
            .finish()
    }
}

impl<'a> Display for DisplayList<'a> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let lineno_width = self.body.iter().fold(0, |max, set| {
            set.display_lines.iter().fold(max, |max, line| match line {
                DisplayLine::Source { lineno, .. } => cmp::max(lineno.unwrap_or(0), max),
                _ => max,
            })
        });
        let lineno_width = if lineno_width == 0 {
            lineno_width
        } else if self.anonymized_line_numbers {
            ANONYMIZED_LINE_NUM.len()
        } else {
            ((lineno_width as f64).log10().floor() as usize) + 1
        };
        let inline_marks_width = self.body.iter().fold(0, |max, set| {
            set.display_lines.iter().fold(max, |max, line| match line {
                DisplayLine::Source { inline_marks, .. } => cmp::max(inline_marks.len(), max),
                _ => max,
            })
        });

        let mut count_offset = 0;
        for set in self.body.iter() {
            self.format_set(set, lineno_width, inline_marks_width, count_offset, f)?;
            count_offset += set.display_lines.len();
        }
        Ok(())
    }
}

impl<'a> DisplayList<'a> {
    pub(crate) fn new(
        message: snippet::Message<'a>,
        stylesheet: &'a Stylesheet,
        anonymized_line_numbers: bool,
        margin: Option<Margin>,
    ) -> DisplayList<'a> {
        let body = format_message(message, margin, true);

        Self {
            body,
            stylesheet,
            anonymized_line_numbers,
        }
    }

    fn format_set(
        &self,
        set: &DisplaySet<'_>,
        lineno_width: usize,
        inline_marks_width: usize,
        count_offset: usize,
        f: &mut fmt::Formatter<'_>,
    ) -> fmt::Result {
        let body_len = self
            .body
            .iter()
            .map(|set| set.display_lines.len())
            .sum::<usize>();
        for (i, line) in set.display_lines.iter().enumerate() {
            set.format_line(
                line,
                lineno_width,
                inline_marks_width,
                self.stylesheet,
                self.anonymized_line_numbers,
                f,
            )?;
            if i + count_offset + 1 < body_len {
                f.write_char('\n')?;
            }
        }
        Ok(())
    }
}

#[derive(Debug, PartialEq)]
pub(crate) struct DisplaySet<'a> {
    pub display_lines: Vec<DisplayLine<'a>>,
    pub margin: Option<Margin>,
}

impl<'a> DisplaySet<'a> {
    fn format_label(
        &self,
        label: &[DisplayTextFragment<'_>],
        stylesheet: &Stylesheet,
        f: &mut fmt::Formatter<'_>,
    ) -> fmt::Result {
        let emphasis_style = stylesheet.emphasis();

        for fragment in label {
            match fragment.style {
                DisplayTextStyle::Regular => fragment.content.fmt(f)?,
                DisplayTextStyle::Emphasis => {
                    write!(
                        f,
                        "{}{}{}",
                        emphasis_style.render(),
                        fragment.content,
                        emphasis_style.render_reset()
                    )?;
                }
            }
        }
        Ok(())
    }

    fn format_annotation(
        &self,
        annotation: &Annotation<'_>,
        continuation: bool,
        in_source: bool,
        stylesheet: &Stylesheet,
        f: &mut fmt::Formatter<'_>,
    ) -> fmt::Result {
        let color = get_annotation_style(&annotation.annotation_type, stylesheet);
        let formatted_len = if let Some(id) = &annotation.id {
            2 + id.len() + annotation_type_len(&annotation.annotation_type)
        } else {
            annotation_type_len(&annotation.annotation_type)
        };

        if continuation {
            format_repeat_char(' ', formatted_len + 2, f)?;
            return self.format_label(&annotation.label, stylesheet, f);
        }
        if formatted_len == 0 {
            self.format_label(&annotation.label, stylesheet, f)
        } else {
            write!(f, "{}", color.render())?;
            format_annotation_type(&annotation.annotation_type, f)?;
            if let Some(id) = &annotation.id {
                f.write_char('[')?;
                f.write_str(id)?;
                f.write_char(']')?;
            }
            write!(f, "{}", color.render_reset())?;

            if !is_annotation_empty(annotation) {
                if in_source {
                    write!(f, "{}", color.render())?;
                    f.write_str(": ")?;
                    self.format_label(&annotation.label, stylesheet, f)?;
                    write!(f, "{}", color.render_reset())?;
                } else {
                    f.write_str(": ")?;
                    self.format_label(&annotation.label, stylesheet, f)?;
                }
            }
            Ok(())
        }
    }

    #[inline]
    fn format_source_line(
        &self,
        line: &DisplaySourceLine<'_>,
        stylesheet: &Stylesheet,
        f: &mut fmt::Formatter<'_>,
    ) -> fmt::Result {
        match line {
            DisplaySourceLine::Empty => Ok(()),
            DisplaySourceLine::Content { text, .. } => {
                f.write_char(' ')?;
                if let Some(margin) = self.margin {
                    let line_len = text.chars().count();
                    let mut left = margin.left(line_len);
                    let right = margin.right(line_len);

                    if margin.was_cut_left() {
                        // We have stripped some code/whitespace from the beginning, make it clear.
                        "...".fmt(f)?;
                        left += 3;
                    }

                    // On long lines, we strip the source line, accounting for unicode.
                    let mut taken = 0;
                    let cut_right = if margin.was_cut_right(line_len) {
                        taken += 3;
                        true
                    } else {
                        false
                    };
                    // Specifies that it will end on the next character, so it will return
                    // until the next one to the final condition.
                    let mut ended = false;
                    let range = text
                        .char_indices()
                        .skip(left)
                        // Complete char iterator with final character
                        .chain(std::iter::once((text.len(), '\0')))
                        // Take until the next one to the final condition
                        .take_while(|(_, ch)| {
                            // Fast return to iterate over final byte position
                            if ended {
                                return false;
                            }
                            // Make sure that the trimming on the right will fall within the terminal width.
                            // FIXME: `unicode_width` sometimes disagrees with terminals on how wide a `char` is.
                            // For now, just accept that sometimes the code line will be longer than desired.
                            taken += unicode_width::UnicodeWidthChar::width(*ch).unwrap_or(1);
                            if taken > right - left {
                                ended = true;
                            }
                            true
                        })
                        // Reduce to start and end byte position
                        .fold((None, 0), |acc, (i, _)| {
                            if acc.0.is_some() {
                                (acc.0, i)
                            } else {
                                (Some(i), i)
                            }
                        });

                    // Format text with margins
                    text[range.0.expect("One character at line")..range.1].fmt(f)?;

                    if cut_right {
                        // We have stripped some code after the right-most span end, make it clear we did so.
                        "...".fmt(f)?;
                    }
                    Ok(())
                } else {
                    text.fmt(f)
                }
            }
            DisplaySourceLine::Annotation {
                range,
                annotation,
                annotation_type,
                annotation_part,
            } => {
                let indent_char = match annotation_part {
                    DisplayAnnotationPart::Standalone => ' ',
                    DisplayAnnotationPart::LabelContinuation => ' ',
                    DisplayAnnotationPart::MultilineStart => '_',
                    DisplayAnnotationPart::MultilineEnd => '_',
                };
                let mark = match annotation_type {
                    DisplayAnnotationType::Error => '^',
                    DisplayAnnotationType::Warning => '-',
                    DisplayAnnotationType::Info => '-',
                    DisplayAnnotationType::Note => '-',
                    DisplayAnnotationType::Help => '-',
                    DisplayAnnotationType::None => ' ',
                };
                let color = get_annotation_style(annotation_type, stylesheet);
                let indent_length = match annotation_part {
                    DisplayAnnotationPart::LabelContinuation => range.1,
                    _ => range.0,
                };

                write!(f, "{}", color.render())?;
                format_repeat_char(indent_char, indent_length + 1, f)?;
                format_repeat_char(mark, range.1 - indent_length, f)?;
                write!(f, "{}", color.render_reset())?;

                if !is_annotation_empty(annotation) {
                    f.write_char(' ')?;
                    write!(f, "{}", color.render())?;
                    self.format_annotation(
                        annotation,
                        annotation_part == &DisplayAnnotationPart::LabelContinuation,
                        true,
                        stylesheet,
                        f,
                    )?;
                    write!(f, "{}", color.render_reset())?;
                }

                Ok(())
            }
        }
    }

    #[inline]
    fn format_raw_line(
        &self,
        line: &DisplayRawLine<'_>,
        lineno_width: usize,
        stylesheet: &Stylesheet,
        f: &mut fmt::Formatter<'_>,
    ) -> fmt::Result {
        match line {
            DisplayRawLine::Origin {
                path,
                pos,
                header_type,
            } => {
                let header_sigil = match header_type {
                    DisplayHeaderType::Initial => "-->",
                    DisplayHeaderType::Continuation => ":::",
                };
                let lineno_color = stylesheet.line_no();

                if let Some((col, row)) = pos {
                    format_repeat_char(' ', lineno_width, f)?;
                    write!(
                        f,
                        "{}{}{}",
                        lineno_color.render(),
                        header_sigil,
                        lineno_color.render_reset()
                    )?;
                    f.write_char(' ')?;
                    path.fmt(f)?;
                    f.write_char(':')?;
                    col.fmt(f)?;
                    f.write_char(':')?;
                    row.fmt(f)
                } else {
                    format_repeat_char(' ', lineno_width, f)?;
                    write!(
                        f,
                        "{}{}{}",
                        lineno_color.render(),
                        header_sigil,
                        lineno_color.render_reset()
                    )?;
                    f.write_char(' ')?;
                    path.fmt(f)
                }
            }
            DisplayRawLine::Annotation {
                annotation,
                source_aligned,
                continuation,
            } => {
                if *source_aligned {
                    if *continuation {
                        format_repeat_char(' ', lineno_width + 3, f)?;
                    } else {
                        let lineno_color = stylesheet.line_no();
                        format_repeat_char(' ', lineno_width, f)?;
                        f.write_char(' ')?;
                        write!(
                            f,
                            "{}={}",
                            lineno_color.render(),
                            lineno_color.render_reset()
                        )?;
                        f.write_char(' ')?;
                    }
                }
                self.format_annotation(annotation, *continuation, false, stylesheet, f)
            }
        }
    }

    #[inline]
    fn format_line(
        &self,
        dl: &DisplayLine<'_>,
        lineno_width: usize,
        inline_marks_width: usize,
        stylesheet: &Stylesheet,
        anonymized_line_numbers: bool,
        f: &mut fmt::Formatter<'_>,
    ) -> fmt::Result {
        match dl {
            DisplayLine::Source {
                lineno,
                inline_marks,
                line,
            } => {
                let lineno_color = stylesheet.line_no();
                if anonymized_line_numbers && lineno.is_some() {
                    write!(f, "{}", lineno_color.render())?;
                    f.write_str(ANONYMIZED_LINE_NUM)?;
                    f.write_str(" |")?;
                    write!(f, "{}", lineno_color.render_reset())?;
                } else {
                    write!(f, "{}", lineno_color.render())?;
                    match lineno {
                        Some(n) => write!(f, "{:>width$}", n, width = lineno_width),
                        None => format_repeat_char(' ', lineno_width, f),
                    }?;
                    f.write_str(" |")?;
                    write!(f, "{}", lineno_color.render_reset())?;
                }
                if *line != DisplaySourceLine::Empty {
                    if !inline_marks.is_empty() || 0 < inline_marks_width {
                        f.write_char(' ')?;
                        self.format_inline_marks(inline_marks, inline_marks_width, stylesheet, f)?;
                    }
                    self.format_source_line(line, stylesheet, f)?;
                } else if !inline_marks.is_empty() {
                    f.write_char(' ')?;
                    self.format_inline_marks(inline_marks, inline_marks_width, stylesheet, f)?;
                }
                Ok(())
            }
            DisplayLine::Fold { inline_marks } => {
                f.write_str("...")?;
                if !inline_marks.is_empty() || 0 < inline_marks_width {
                    format_repeat_char(' ', lineno_width, f)?;
                    self.format_inline_marks(inline_marks, inline_marks_width, stylesheet, f)?;
                }
                Ok(())
            }
            DisplayLine::Raw(line) => self.format_raw_line(line, lineno_width, stylesheet, f),
        }
    }

    fn format_inline_marks(
        &self,
        inline_marks: &[DisplayMark],
        inline_marks_width: usize,
        stylesheet: &Stylesheet,
        f: &mut fmt::Formatter<'_>,
    ) -> fmt::Result {
        format_repeat_char(' ', inline_marks_width - inline_marks.len(), f)?;
        for mark in inline_marks {
            let annotation_style = get_annotation_style(&mark.annotation_type, stylesheet);
            write!(f, "{}", annotation_style.render())?;
            f.write_char(match mark.mark_type {
                DisplayMarkType::AnnotationThrough => '|',
                DisplayMarkType::AnnotationStart => '/',
            })?;
            write!(f, "{}", annotation_style.render_reset())?;
        }
        Ok(())
    }
}

/// Inline annotation which can be used in either Raw or Source line.
#[derive(Debug, PartialEq)]
pub struct Annotation<'a> {
    pub annotation_type: DisplayAnnotationType,
    pub id: Option<&'a str>,
    pub label: Vec<DisplayTextFragment<'a>>,
}

/// A single line used in `DisplayList`.
#[derive(Debug, PartialEq)]
pub enum DisplayLine<'a> {
    /// A line with `lineno` portion of the slice.
    Source {
        lineno: Option<usize>,
        inline_marks: Vec<DisplayMark>,
        line: DisplaySourceLine<'a>,
    },

    /// A line indicating a folded part of the slice.
    Fold { inline_marks: Vec<DisplayMark> },

    /// A line which is displayed outside of slices.
    Raw(DisplayRawLine<'a>),
}

/// A source line.
#[derive(Debug, PartialEq)]
pub enum DisplaySourceLine<'a> {
    /// A line with the content of the Snippet.
    Content {
        text: &'a str,
        range: (usize, usize), // meta information for annotation placement.
    },

    /// An annotation line which is displayed in context of the slice.
    Annotation {
        annotation: Annotation<'a>,
        range: (usize, usize),
        annotation_type: DisplayAnnotationType,
        annotation_part: DisplayAnnotationPart,
    },

    /// An empty source line.
    Empty,
}

/// Raw line - a line which does not have the `lineno` part and is not considered
/// a part of the snippet.
#[derive(Debug, PartialEq)]
pub enum DisplayRawLine<'a> {
    /// A line which provides information about the location of the given
    /// slice in the project structure.
    Origin {
        path: &'a str,
        pos: Option<(usize, usize)>,
        header_type: DisplayHeaderType,
    },

    /// An annotation line which is not part of any snippet.
    Annotation {
        annotation: Annotation<'a>,

        /// If set to `true`, the annotation will be aligned to the
        /// lineno delimiter of the snippet.
        source_aligned: bool,
        /// If set to `true`, only the label of the `Annotation` will be
        /// displayed. It allows for a multiline annotation to be aligned
        /// without displaying the meta information (`type` and `id`) to be
        /// displayed on each line.
        continuation: bool,
    },
}

/// An inline text fragment which any label is composed of.
#[derive(Debug, PartialEq)]
pub struct DisplayTextFragment<'a> {
    pub content: &'a str,
    pub style: DisplayTextStyle,
}

/// A style for the `DisplayTextFragment` which can be visually formatted.
///
/// This information may be used to emphasis parts of the label.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DisplayTextStyle {
    Regular,
    Emphasis,
}

/// An indicator of what part of the annotation a given `Annotation` is.
#[derive(Debug, Clone, PartialEq)]
pub enum DisplayAnnotationPart {
    /// A standalone, single-line annotation.
    Standalone,
    /// A continuation of a multi-line label of an annotation.
    LabelContinuation,
    /// A line starting a multiline annotation.
    MultilineStart,
    /// A line ending a multiline annotation.
    MultilineEnd,
}

/// A visual mark used in `inline_marks` field of the `DisplaySourceLine`.
#[derive(Debug, Clone, PartialEq)]
pub struct DisplayMark {
    pub mark_type: DisplayMarkType,
    pub annotation_type: DisplayAnnotationType,
}

/// A type of the `DisplayMark`.
#[derive(Debug, Clone, PartialEq)]
pub enum DisplayMarkType {
    /// A mark indicating a multiline annotation going through the current line.
    AnnotationThrough,
    /// A mark indicating a multiline annotation starting on the given line.
    AnnotationStart,
}

/// A type of the `Annotation` which may impact the sigils, style or text displayed.
///
/// There are several ways to uses this information when formatting the `DisplayList`:
///
/// * An annotation may display the name of the type like `error` or `info`.
/// * An underline for `Error` may be `^^^` while for `Warning` it could be `---`.
/// * `ColorStylesheet` may use different colors for different annotations.
#[derive(Debug, Clone, PartialEq)]
pub enum DisplayAnnotationType {
    None,
    Error,
    Warning,
    Info,
    Note,
    Help,
}

impl From<snippet::Level> for DisplayAnnotationType {
    fn from(at: snippet::Level) -> Self {
        match at {
            snippet::Level::Error => DisplayAnnotationType::Error,
            snippet::Level::Warning => DisplayAnnotationType::Warning,
            snippet::Level::Info => DisplayAnnotationType::Info,
            snippet::Level::Note => DisplayAnnotationType::Note,
            snippet::Level::Help => DisplayAnnotationType::Help,
        }
    }
}

/// Information whether the header is the initial one or a consequitive one
/// for multi-slice cases.
// TODO: private
#[derive(Debug, Clone, PartialEq)]
pub enum DisplayHeaderType {
    /// Initial header is the first header in the snippet.
    Initial,

    /// Continuation marks all headers of following slices in the snippet.
    Continuation,
}

struct CursorLines<'a>(&'a str);

impl<'a> CursorLines<'a> {
    fn new(src: &str) -> CursorLines<'_> {
        CursorLines(src)
    }
}

enum EndLine {
    Eof = 0,
    Crlf = 1,
    Lf = 2,
}

impl<'a> Iterator for CursorLines<'a> {
    type Item = (&'a str, EndLine);

    fn next(&mut self) -> Option<Self::Item> {
        if self.0.is_empty() {
            None
        } else {
            self.0
                .find('\n')
                .map(|x| {
                    let ret = if 0 < x {
                        if self.0.as_bytes()[x - 1] == b'\r' {
                            (&self.0[..x - 1], EndLine::Lf)
                        } else {
                            (&self.0[..x], EndLine::Crlf)
                        }
                    } else {
                        ("", EndLine::Crlf)
                    };
                    self.0 = &self.0[x + 1..];
                    ret
                })
                .or_else(|| {
                    let ret = Some((self.0, EndLine::Eof));
                    self.0 = "";
                    ret
                })
        }
    }
}

fn format_message(
    snippet::Message {
        level,
        id,
        title,
        footer,
        snippets,
    }: snippet::Message<'_>,
    margin: Option<Margin>,
    primary: bool,
) -> Vec<DisplaySet<'_>> {
    let mut sets = vec![];
    let body = if !snippets.is_empty() || primary {
        vec![format_title(level, id, title)]
    } else {
        format_footer(level, id, title)
    };

    for (idx, snippet) in snippets.into_iter().enumerate() {
        sets.push(format_snippet(
            snippet,
            idx == 0,
            !footer.is_empty(),
            margin,
        ));
    }

    if let Some(first) = sets.first_mut() {
        body.into_iter().for_each(|line| {
            first.display_lines.insert(0, line);
        });
    } else {
        sets.push(DisplaySet {
            display_lines: body,
            margin,
        });
    }

    for annotation in footer {
        sets.extend(format_message(annotation, margin, false));
    }

    sets
}

fn format_title<'a>(level: crate::Level, id: Option<&'a str>, label: &'a str) -> DisplayLine<'a> {
    DisplayLine::Raw(DisplayRawLine::Annotation {
        annotation: Annotation {
            annotation_type: DisplayAnnotationType::from(level),
            id,
            label: format_label(Some(label), Some(DisplayTextStyle::Emphasis)),
        },
        source_aligned: false,
        continuation: false,
    })
}

fn format_footer<'a>(
    level: crate::Level,
    id: Option<&'a str>,
    label: &'a str,
) -> Vec<DisplayLine<'a>> {
    let mut result = vec![];
    for (i, line) in label.lines().enumerate() {
        result.push(DisplayLine::Raw(DisplayRawLine::Annotation {
            annotation: Annotation {
                annotation_type: DisplayAnnotationType::from(level),
                id,
                label: format_label(Some(line), None),
            },
            source_aligned: true,
            continuation: i != 0,
        }));
    }
    result
}

fn format_label(
    label: Option<&str>,
    style: Option<DisplayTextStyle>,
) -> Vec<DisplayTextFragment<'_>> {
    let mut result = vec![];
    if let Some(label) = label {
        let element_style = style.unwrap_or(DisplayTextStyle::Regular);
        result.push(DisplayTextFragment {
            content: label,
            style: element_style,
        });
    }
    result
}

fn format_snippet(
    snippet: snippet::Snippet<'_>,
    is_first: bool,
    has_footer: bool,
    margin: Option<Margin>,
) -> DisplaySet<'_> {
    let main_range = snippet.annotations.first().map(|x| x.range.start);
    let origin = snippet.origin;
    let need_empty_header = origin.is_some() || is_first;
    let body = format_body(snippet, need_empty_header, has_footer, margin);
    let header = format_header(origin, main_range, &body, is_first);
    let mut result = vec![];

    if let Some(header) = header {
        result.push(header);
    }
    result.extend(body);
    DisplaySet {
        display_lines: result,
        margin,
    }
}

#[inline]
// TODO: option_zip
fn zip_opt<A, B>(a: Option<A>, b: Option<B>) -> Option<(A, B)> {
    a.and_then(|a| b.map(|b| (a, b)))
}

fn format_header<'a>(
    origin: Option<&'a str>,
    main_range: Option<usize>,
    body: &[DisplayLine<'_>],
    is_first: bool,
) -> Option<DisplayLine<'a>> {
    let display_header = if is_first {
        DisplayHeaderType::Initial
    } else {
        DisplayHeaderType::Continuation
    };

    if let Some((main_range, path)) = zip_opt(main_range, origin) {
        let mut col = 1;
        let mut line_offset = 1;

        for item in body {
            if let DisplayLine::Source {
                line: DisplaySourceLine::Content { text, range },
                lineno,
                ..
            } = item
            {
                if main_range >= range.0 && main_range <= range.1 {
                    let char_column = text[0..(main_range - range.0)].chars().count();
                    col = char_column + 1;
                    line_offset = lineno.unwrap_or(1);
                    break;
                }
            }
        }

        return Some(DisplayLine::Raw(DisplayRawLine::Origin {
            path,
            pos: Some((line_offset, col)),
            header_type: display_header,
        }));
    }

    if let Some(path) = origin {
        return Some(DisplayLine::Raw(DisplayRawLine::Origin {
            path,
            pos: None,
            header_type: display_header,
        }));
    }

    None
}

fn fold_body(mut body: Vec<DisplayLine<'_>>) -> Vec<DisplayLine<'_>> {
    enum Line {
        Fold(usize),
        Source(usize),
    }

    let mut lines = vec![];
    let mut no_annotation_lines_counter = 0;

    for (idx, line) in body.iter().enumerate() {
        match line {
            DisplayLine::Source {
                line: DisplaySourceLine::Annotation { .. },
                ..
            } => {
                let fold_start = idx - no_annotation_lines_counter;
                if no_annotation_lines_counter > 2 {
                    let fold_end = idx;
                    let pre_len = if no_annotation_lines_counter > 8 {
                        4
                    } else {
                        0
                    };
                    let post_len = if no_annotation_lines_counter > 8 {
                        2
                    } else {
                        1
                    };
                    for (i, _) in body
                        .iter()
                        .enumerate()
                        .take(fold_start + pre_len)
                        .skip(fold_start)
                    {
                        lines.push(Line::Source(i));
                    }
                    lines.push(Line::Fold(idx));
                    for (i, _) in body
                        .iter()
                        .enumerate()
                        .take(fold_end)
                        .skip(fold_end - post_len)
                    {
                        lines.push(Line::Source(i));
                    }
                } else {
                    for (i, _) in body.iter().enumerate().take(idx).skip(fold_start) {
                        lines.push(Line::Source(i));
                    }
                }
                no_annotation_lines_counter = 0;
            }
            DisplayLine::Source { .. } => {
                no_annotation_lines_counter += 1;
                continue;
            }
            _ => {
                no_annotation_lines_counter += 1;
            }
        }
        lines.push(Line::Source(idx));
    }

    let mut new_body = vec![];
    let mut removed = 0;
    for line in lines {
        match line {
            Line::Source(i) => {
                new_body.push(body.remove(i - removed));
                removed += 1;
            }
            Line::Fold(i) => {
                if let DisplayLine::Source {
                    line: DisplaySourceLine::Annotation { .. },
                    ref inline_marks,
                    ..
                } = body.get(i - removed).unwrap()
                {
                    new_body.push(DisplayLine::Fold {
                        inline_marks: inline_marks.clone(),
                    })
                } else {
                    unreachable!()
                }
            }
        }
    }

    new_body
}

fn format_body(
    snippet: snippet::Snippet<'_>,
    need_empty_header: bool,
    has_footer: bool,
    margin: Option<Margin>,
) -> Vec<DisplayLine<'_>> {
    let source_len = snippet.source.len();
    if let Some(bigger) = snippet.annotations.iter().find_map(|x| {
        // Allow highlighting one past the last character in the source.
        if source_len + 1 < x.range.end {
            Some(&x.range)
        } else {
            None
        }
    }) {
        panic!(
            "SourceAnnotation range `{:?}` is beyond the end of buffer `{}`",
            bigger, source_len
        )
    }

    let mut body = vec![];
    let mut current_line = snippet.line_start;
    let mut current_index = 0;

    let mut annotation_line_count = 0;
    let mut annotations = snippet.annotations;
    for (idx, (line, end_line)) in CursorLines::new(snippet.source).enumerate() {
        let line_length: usize = line.len();
        let line_range = (current_index, current_index + line_length);
        body.push(DisplayLine::Source {
            lineno: Some(current_line),
            inline_marks: vec![],
            line: DisplaySourceLine::Content {
                text: line,
                range: line_range,
            },
        });
        let line_start_index = line_range.0;
        let line_end_index = line_range.1;
        current_line += 1;
        current_index += line_length + end_line as usize;

        let margin_left = margin
            .map(|m| m.left(line_end_index - line_start_index))
            .unwrap_or_default();
        // It would be nice to use filter_drain here once it's stable.
        annotations.retain(|annotation| {
            let body_idx = idx + annotation_line_count;
            let annotation_type = match annotation.level {
                snippet::Level::Error => DisplayAnnotationType::None,
                snippet::Level::Warning => DisplayAnnotationType::None,
                _ => DisplayAnnotationType::from(annotation.level),
            };
            match annotation.range {
                Range { start, .. } if start > line_end_index => true,
                Range { start, end }
                    if start >= line_start_index && end <= line_end_index
                        || start == line_end_index && end - start <= 1 =>
                {
                    let annotation_start_col = line[0..(start - line_start_index)]
                        .chars()
                        .map(|c| unicode_width::UnicodeWidthChar::width(c).unwrap_or(0))
                        .sum::<usize>()
                        - margin_left;
                    // This allows for annotations to be placed one past the
                    // last character
                    let safe_end = (end - line_start_index).saturating_sub(line_length);
                    let annotation_end_col = line[0..(end - line_start_index) - safe_end]
                        .chars()
                        .map(|c| unicode_width::UnicodeWidthChar::width(c).unwrap_or(0))
                        .sum::<usize>()
                        + safe_end
                        - margin_left;
                    let range = (annotation_start_col, annotation_end_col);
                    body.insert(
                        body_idx + 1,
                        DisplayLine::Source {
                            lineno: None,
                            inline_marks: vec![],
                            line: DisplaySourceLine::Annotation {
                                annotation: Annotation {
                                    annotation_type,
                                    id: None,
                                    label: format_label(annotation.label, None),
                                },
                                range,
                                annotation_type: DisplayAnnotationType::from(annotation.level),
                                annotation_part: DisplayAnnotationPart::Standalone,
                            },
                        },
                    );
                    annotation_line_count += 1;
                    false
                }
                Range { start, end }
                    if start >= line_start_index
                        && start <= line_end_index
                        && end > line_end_index =>
                {
                    if start - line_start_index == 0 {
                        if let DisplayLine::Source {
                            ref mut inline_marks,
                            ..
                        } = body[body_idx]
                        {
                            inline_marks.push(DisplayMark {
                                mark_type: DisplayMarkType::AnnotationStart,
                                annotation_type: DisplayAnnotationType::from(annotation.level),
                            });
                        }
                    } else {
                        let annotation_start_col = line[0..(start - line_start_index)]
                            .chars()
                            .map(|c| unicode_width::UnicodeWidthChar::width(c).unwrap_or(0))
                            .sum::<usize>();
                        let range = (annotation_start_col, annotation_start_col + 1);
                        body.insert(
                            body_idx + 1,
                            DisplayLine::Source {
                                lineno: None,
                                inline_marks: vec![],
                                line: DisplaySourceLine::Annotation {
                                    annotation: Annotation {
                                        annotation_type: DisplayAnnotationType::None,
                                        id: None,
                                        label: vec![],
                                    },
                                    range,
                                    annotation_type: DisplayAnnotationType::from(annotation.level),
                                    annotation_part: DisplayAnnotationPart::MultilineStart,
                                },
                            },
                        );
                        annotation_line_count += 1;
                    }
                    true
                }
                Range { start, end } if start < line_start_index && end > line_end_index => {
                    if let DisplayLine::Source {
                        ref mut inline_marks,
                        ..
                    } = body[body_idx]
                    {
                        inline_marks.push(DisplayMark {
                            mark_type: DisplayMarkType::AnnotationThrough,
                            annotation_type: DisplayAnnotationType::from(annotation.level),
                        });
                    }
                    true
                }
                Range { start, end }
                    if start < line_start_index
                        && end >= line_start_index
                        && end <= line_end_index =>
                {
                    if let DisplayLine::Source {
                        ref mut inline_marks,
                        ..
                    } = body[body_idx]
                    {
                        inline_marks.push(DisplayMark {
                            mark_type: DisplayMarkType::AnnotationThrough,
                            annotation_type: DisplayAnnotationType::from(annotation.level),
                        });
                    }

                    let end_mark = line[0..(end - line_start_index)]
                        .chars()
                        .map(|c| unicode_width::UnicodeWidthChar::width(c).unwrap_or(0))
                        .sum::<usize>()
                        .saturating_sub(1);
                    let range = (end_mark - margin_left, (end_mark + 1) - margin_left);
                    body.insert(
                        body_idx + 1,
                        DisplayLine::Source {
                            lineno: None,
                            inline_marks: vec![DisplayMark {
                                mark_type: DisplayMarkType::AnnotationThrough,
                                annotation_type: DisplayAnnotationType::from(annotation.level),
                            }],
                            line: DisplaySourceLine::Annotation {
                                annotation: Annotation {
                                    annotation_type,
                                    id: None,
                                    label: format_label(annotation.label, None),
                                },
                                range,
                                annotation_type: DisplayAnnotationType::from(annotation.level),
                                annotation_part: DisplayAnnotationPart::MultilineEnd,
                            },
                        },
                    );
                    annotation_line_count += 1;
                    false
                }
                _ => true,
            }
        });
    }

    if snippet.fold {
        body = fold_body(body);
    }

    if need_empty_header {
        body.insert(
            0,
            DisplayLine::Source {
                lineno: None,
                inline_marks: vec![],
                line: DisplaySourceLine::Empty,
            },
        );
    }

    if has_footer {
        body.push(DisplayLine::Source {
            lineno: None,
            inline_marks: vec![],
            line: DisplaySourceLine::Empty,
        });
    } else if let Some(DisplayLine::Source { .. }) = body.last() {
        body.push(DisplayLine::Source {
            lineno: None,
            inline_marks: vec![],
            line: DisplaySourceLine::Empty,
        });
    }
    body
}

fn format_repeat_char(c: char, n: usize, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    for _ in 0..n {
        f.write_char(c)?;
    }
    Ok(())
}

#[inline]
fn format_annotation_type(
    annotation_type: &DisplayAnnotationType,
    f: &mut fmt::Formatter<'_>,
) -> fmt::Result {
    match annotation_type {
        DisplayAnnotationType::Error => f.write_str(ERROR_TXT),
        DisplayAnnotationType::Help => f.write_str(HELP_TXT),
        DisplayAnnotationType::Info => f.write_str(INFO_TXT),
        DisplayAnnotationType::Note => f.write_str(NOTE_TXT),
        DisplayAnnotationType::Warning => f.write_str(WARNING_TXT),
        DisplayAnnotationType::None => Ok(()),
    }
}

fn annotation_type_len(annotation_type: &DisplayAnnotationType) -> usize {
    match annotation_type {
        DisplayAnnotationType::Error => ERROR_TXT.len(),
        DisplayAnnotationType::Help => HELP_TXT.len(),
        DisplayAnnotationType::Info => INFO_TXT.len(),
        DisplayAnnotationType::Note => NOTE_TXT.len(),
        DisplayAnnotationType::Warning => WARNING_TXT.len(),
        DisplayAnnotationType::None => 0,
    }
}

fn get_annotation_style<'a>(
    annotation_type: &DisplayAnnotationType,
    stylesheet: &'a Stylesheet,
) -> &'a Style {
    match annotation_type {
        DisplayAnnotationType::Error => stylesheet.error(),
        DisplayAnnotationType::Warning => stylesheet.warning(),
        DisplayAnnotationType::Info => stylesheet.info(),
        DisplayAnnotationType::Note => stylesheet.note(),
        DisplayAnnotationType::Help => stylesheet.help(),
        DisplayAnnotationType::None => stylesheet.none(),
    }
}

#[inline]
fn is_annotation_empty(annotation: &Annotation<'_>) -> bool {
    annotation
        .label
        .iter()
        .all(|fragment| fragment.content.is_empty())
}
