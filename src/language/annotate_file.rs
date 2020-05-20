use std::{
    collections::HashMap,
    fs::File,
    io::{self, Read},
    path::PathBuf,
};

use crate::{config::Config, language::syntax::SyntaxCounter, utils::ext::SliceExt};

use encoding_rs_io::DecodeReaderBytesBuilder;
use grep_searcher::LineIter;
use rayon::prelude::*;

use crate::LanguageType;

/// Type of Line
#[derive(Debug)]
pub enum LineType {
    /// Blank line
    Blank,
    /// Line of code
    Code,
    /// Comment line
    Comment,
}

impl LanguageType {
    /// Annotates a given `Path` using the `LanguageType`. Returning `HashMap<usize, LineType>`
    /// on success and giving back ownership of PathBuf on error.
    pub fn annotate_file<P: Into<PathBuf>>(
        self,
        path: P,
        config: &Config,
    ) -> Result<HashMap<usize, LineType>, (io::Error, PathBuf)> {
        let path = path.into();
        let text = {
            let f = match File::open(&path) {
                Ok(f) => f,
                Err(e) => return Err((e, path)),
            };
            let mut s = Vec::new();
            let mut reader = DecodeReaderBytesBuilder::new().build(f);

            if let Err(e) = reader.read_to_end(&mut s) {
                return Err((e, path));
            }
            s
        };

        Ok(self.annotate_from_slice(&text, config))
    }

    /// Parses the text provided. Returning `HashMap<usize, LineType>` on success.
    pub fn annotate_from_slice<A: AsRef<[u8]>>(
        self,
        text: A,
        config: &Config,
    ) -> HashMap<usize, LineType> {
        let text = text.as_ref();
        let lines = LineIter::new(b'\n', text);
        let mut annotations: HashMap<usize, LineType> = HashMap::new();
        let syntax = SyntaxCounter::new(self);

        if self.is_blank() {
            lines.enumerate().for_each(|(num, _)| {
                annotations.insert(num, LineType::Code);
            });
            annotations
        } else if let Some(end) = syntax
            .shared
            .important_syntax
            .earliest_find(text)
            .and_then(|m| {
                // Get the position of the last line before the important
                // syntax.
                text[..=m.start()]
                    .into_iter()
                    .rev()
                    .position(|&c| c == b'\n')
                    .filter(|&p| p != 0)
                    .map(|p| m.start() - p)
            })
        {
            let (skippable_text, rest) = text.split_at(end + 1);
            let lines = LineIter::new(b'\n', skippable_text);
            let is_fortran = syntax.shared.is_fortran;
            let comments = syntax.shared.line_comments;

            let (mut annots_first, annots_last) = rayon::join(
                move || {
                    self.annotate_lines(config, LineIter::new(b'\n', rest), annotations, syntax)
                },
                move || {
                    lines
                        .enumerate()
                        .par_bridge()
                        .map(|(num, line)| {
                            // FORTRAN has a rule where it only counts as a comment if it's the
                            // first character in the column, so removing starting whitespace
                            // could cause a miscount.
                            let mut line_map: HashMap<usize, LineType> = HashMap::new();
                            let line = if is_fortran { line } else { line.trim() };
                            trace!("{}", String::from_utf8_lossy(line));

                            if line.trim().is_empty() {
                                line_map.insert(num, LineType::Blank);
                            } else if comments.iter().any(|c| line.starts_with(c.as_bytes())) {
                                line_map.insert(num, LineType::Comment);
                            } else {
                                line_map.insert(num, LineType::Code);
                            }
                            line_map
                        })
                        .reduce(
                            || HashMap::new(),
                            |mut annotations, map| -> HashMap<usize, LineType> {
                                annotations.extend(map);
                                annotations
                            },
                        )
                },
            );

            // combine all HashMaps together, no overlap occurs
            annots_first.extend(annots_last);
            annots_first
        } else {
            self.annotate_lines(config, lines, annotations, syntax)
        }
    }

    #[inline]
    fn annotate_lines<'a>(
        self,
        config: &Config,
        lines: impl IntoIterator<Item = &'a [u8]>,
        mut annotations: HashMap<usize, LineType>,
        mut syntax: SyntaxCounter,
    ) -> HashMap<usize, LineType> {
        for (line_num, line) in lines.into_iter().enumerate() {
            // FORTRAN has a rule where it only counts as a comment if it's the
            // first character in the column, so removing starting whitespace
            // could cause a miscount.
            let line = if syntax.shared.is_fortran {
                line
            } else {
                line.trim()
            };
            trace!("{}", String::from_utf8_lossy(line));

            if line.trim().is_empty() {
                annotations.insert(line_num, LineType::Blank);
                trace!("Blank on Line No.{}", line_num);
                continue;
            } else if syntax.is_plain_mode() && !syntax.shared.important_syntax.is_match(line) {
                trace!("^ Skippable");

                if syntax
                    .shared
                    .line_comments
                    .iter()
                    .any(|c| line.starts_with(c.as_bytes()))
                {
                    annotations.insert(line_num, LineType::Comment);
                    trace!("Comment on Line No.{}", line_num);
                } else {
                    annotations.insert(line_num, LineType::Code);
                    trace!("Code on Line No.{}", line_num);
                }
                continue;
            }

            let had_multi_line = !syntax.stack.is_empty();
            let mut ended_with_comments = false;
            let mut skip = 0;
            macro_rules! skip {
                ($skip:expr) => {{
                    skip = $skip - 1;
                }};
            }

            'window: for i in 0..line.len() {
                if skip != 0 {
                    skip -= 1;
                    continue;
                }

                ended_with_comments = false;
                let window = &line[i..];

                let is_end_of_quote_or_multi_line = syntax
                    .parse_end_of_quote(window)
                    .or_else(|| syntax.parse_end_of_multi_line(window));

                if let Some(skip_amount) = is_end_of_quote_or_multi_line {
                    ended_with_comments = true;
                    skip!(skip_amount);
                    continue;
                } else if syntax.quote.is_some() {
                    continue;
                }

                let is_quote_or_multi_line = syntax
                    .parse_quote(window)
                    .or_else(|| syntax.parse_multi_line_comment(window));

                if let Some(skip_amount) = is_quote_or_multi_line {
                    skip!(skip_amount);
                    continue;
                }

                if syntax.parse_line_comment(window) {
                    ended_with_comments = true;
                    break 'window;
                }
            }

            trace!("{}", String::from_utf8_lossy(line));

            let is_comments = ((!syntax.stack.is_empty() || ended_with_comments) && had_multi_line)
                || (
                    // If we're currently in a comment or we just ended
                    // with one.
                    syntax.shared.any_comments.is_match(line) && syntax.quote.is_none()
                )
                || ((
                        // If we're currently in a doc string or we just ended
                        // with one.
                        syntax.quote.is_some() ||
                        syntax.shared.doc_quotes.iter().any(|(s, _)| line.starts_with(s.as_bytes()))
                    ) &&
                    // `Some(true)` is import in order to respect the current
                    // configuration.
                    config.treat_doc_strings_as_comments == Some(true) &&
                    syntax.quote_is_doc_quote);

            if is_comments {
                annotations.insert(line_num, LineType::Comment);
                trace!("Comment on Line No.{}", line_num);
                trace!("Was the Comment stack empty?: {}", !had_multi_line);
            } else {
                annotations.insert(line_num, LineType::Code);
                trace!("Code on Line No.{}", line_num);
            }
        }

        annotations
    }
}
