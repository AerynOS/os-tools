// SPDX-FileCopyrightText: 2023 AerynOS Developers
// SPDX-License-Identifier: MPL-2.0

use std::ops;

/// Apply update operations to a yaml file
#[derive(Debug, Default)]
pub struct Updater {
    operations: Vec<Operation>,
}

impl Updater {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn update_key(&mut self, key: impl ToString, f: impl FnOnce(Path) -> Path) {
        let path = f(Path::default());

        self.operations.push(Operation {
            path,
            update: Update::Key(key.to_string()),
        });
    }

    pub fn update_value(&mut self, value: impl ToString, f: impl FnOnce(Path) -> Path) {
        let path = f(Path::default());

        self.operations.push(Operation {
            path,
            update: Update::Value(value.to_string()),
        });
    }

    pub fn apply(&self, input: impl ToString) -> String {
        let mut output = self
            .operations
            .iter()
            .fold(input.to_string(), |input, operation| operation.apply(&input));
        if !output.ends_with('\n') {
            output.push('\n');
        }
        output
    }
}

#[derive(Debug, Clone)]
enum Segment {
    Sequence(usize),
    Map(String),
}

#[derive(Debug, Clone, Default)]
pub struct Path(Vec<Segment>);

impl ops::Div<usize> for Path {
    type Output = Self;

    fn div(self, rhs: usize) -> Self::Output {
        Self(self.0.into_iter().chain(Some(Segment::Sequence(rhs))).collect())
    }
}

impl<'a> ops::Div<&'a str> for Path {
    type Output = Self;

    fn div(self, rhs: &'a str) -> Self::Output {
        Self(self.0.into_iter().chain(Some(Segment::Map(rhs.to_owned()))).collect())
    }
}

#[derive(Debug)]
enum Update {
    Key(String),
    Value(String),
}

#[derive(Debug)]
struct Operation {
    path: Path,
    update: Update,
}

#[derive(Debug)]
struct Substr {
    start: usize,
    end: usize,
}

impl Substr {
    fn value<'a>(&self, line: &'a str) -> &'a str {
        &line[self.start..=self.end]
    }

    fn range(&self) -> ops::Range<usize> {
        self.start..self.end + 1
    }
}

fn sequence_scalar(line: &str) -> Option<Substr> {
    // Start is first non-whitespace char after `- `
    let start = line.find("- ")? + 2;
    let start = start + line[start..].find(|c: char| !c.is_whitespace())?;

    // Ensure this isn't a non-scalar value
    if line[start..].starts_with(':') || line[start..].starts_with('-') {
        return None;
    }

    // End is first non-whitespace character from the right
    //  after comment or end
    let end = start + line[start..].rfind(" #").unwrap_or(line[start..].len());
    let end = start + line[start..end].rfind(|c: char| !c.is_whitespace())?;

    Some(Substr { start, end })
}

fn map_key_scalar(line: &str) -> Option<Substr> {
    // Start is first non-whitespace char after `- ` or whitespace
    let start = line.find("- ").map(|i| i + 2).unwrap_or(0);
    let start = start + line[start..].find(|c: char| !c.is_whitespace())?;

    // End is first non-whitespace character from the right
    // before `: ` or ending `:`
    let end = start
        + line[start..]
            .find(": ")
            .or_else(|| line[start..].ends_with(':').then_some(line[start..].len() - 1))?;
    let end = start + line[start..end].rfind(|c: char| !c.is_whitespace())?;

    Some(Substr { start, end })
}

fn map_value_scalar(line: &str) -> Option<Substr> {
    let key = map_key_scalar(line)?;

    // Start is first non-whitespace char after key scalar then `: `
    let start = key.end + line[key.end..].find(": ")? + 2;
    let start = start + line[start..].find(|c: char| !c.is_whitespace())?;

    // End is first non-whitespace character from the right
    // after comment or end
    let end = start + line[start..].rfind(" #").unwrap_or(line[start..].len());
    let end = start + line[start..end].rfind(|c: char| !c.is_whitespace())?;

    Some(Substr { start, end })
}

impl Operation {
    fn apply(&self, source: &str) -> String {
        let mut lines = source.lines().map(String::from).collect::<Vec<_>>();
        let segments = self.path.0.iter().enumerate();

        // Keep track of which sequence item we're on
        let mut sequence_index = 0;
        // If match is found
        let mut matched_substr = None;
        // Updated w/ current nesting level and referenced
        // to prevent looking at lines with a smaller indent
        // (only move down, never back up)
        let mut current_indent = 0;
        // What line are we checking
        let mut current_line = 0;

        // Calculate indent level
        let indent = |line: &str| line.len() - line.trim_start().len();

        // For each segment, seek over lines until a matching line
        // is found for the segment, then proceed to checking the next
        // segment.
        //
        // If it's the last segment, that substring is used for the replacement
        for (segment_idx, segment) in segments {
            let is_last_segment = segment_idx == self.path.0.len() - 1;

            while current_line < lines.len() {
                let line = &lines[current_line];

                // Prevent bubbling back up the yaml document if
                // a match isn't found at this level, to prevent
                // matching at higher levels which don't match the
                // walked path
                let indent = indent(line);
                if indent < current_indent {
                    break;
                }

                match segment {
                    Segment::Sequence(i) => {
                        // Are we on a sequence line?
                        if let Some(substr) = sequence_scalar(line) {
                            // Does it match the desired index?
                            if *i == sequence_index {
                                // If last set the match
                                if is_last_segment {
                                    matched_substr = Some((current_line, substr));
                                }
                                current_indent = indent;
                                // We don't increment line count since a map
                                // can exist on same line as a sequence
                                break;
                            } else {
                                sequence_index += 1;
                            }
                        }
                    }
                    Segment::Map(key) => {
                        // Are we on a map line?
                        if let Some(key_substr) = map_key_scalar(line) {
                            // Is it the key we want
                            if key_substr.value(line) == key {
                                if is_last_segment {
                                    match self.update {
                                        Update::Key(_) => matched_substr = Some((current_line, key_substr)),
                                        Update::Value(_) => {
                                            if let Some(value_substr) = map_value_scalar(line) {
                                                matched_substr = Some((current_line, value_substr));
                                            }
                                        }
                                    }
                                }
                                current_indent = indent;
                                current_line += 1;
                                break;
                            }
                        }
                    }
                }

                current_line += 1;
            }
        }

        if let Some((idx, substr)) = matched_substr {
            let line = &mut lines[idx];

            let replacement = match &self.update {
                Update::Key(key) => key,
                Update::Value(value) => value,
            };

            line.replace_range(substr.range(), replacement);
        }

        // Add trailing line-break
        if source.ends_with('\n') {
            lines.push(String::new());
        }

        lines.join("\n")
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_update() {
        let raw = r#"
test : asdf
some:
 - a  # foo
 - b: # bar
     nested: value    # baz
     other:
       asdf: 0  # a comment
"#;
        let expected = r#"
test : 1
some:
 - 2  # foo
 - 3: # bar
     4: 5    # baz
     other:
       asdf: 6  # a comment
"#;

        let mut updater = Updater::new();
        updater.update_value(1, |p| p / "test");
        updater.update_value(2, |p| p / "some" / 0);
        // Define nested updates in reverse order since they're parsed sequentially
        updater.update_value(6, |p| p / "some" / 1 / "other" / "asdf");
        updater.update_value(5, |p| p / "some" / 1 / "nested");
        updater.update_key(4, |p| p / "some" / 1 / "nested");
        updater.update_key(3, |p| p / "some" / 1 / "b");

        let actual = updater.apply(raw);

        assert_eq!(actual, expected);
    }
}
