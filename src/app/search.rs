use crate::model::ChangedFile;

pub(crate) fn fuzzy_match(query: &str, candidate: &str) -> bool {
    if query.is_empty() {
        return true;
    }
    let mut candidate = candidate.chars().flat_map(char::to_lowercase);
    query
        .chars()
        .flat_map(char::to_lowercase)
        .all(|query_character| {
            candidate.any(|candidate_character| candidate_character == query_character)
        })
}

pub(crate) fn diff_matches(file: Option<&ChangedFile>, query: &str) -> Vec<usize> {
    let Some(file) = file else {
        return Vec::new();
    };
    let query = query.to_lowercase();
    let mut matches = Vec::new();
    let mut row_index = 0;
    for hunk in &file.hunks {
        if hunk.header.to_lowercase().contains(&query) {
            matches.push(row_index);
        }
        row_index += 1;
        if !hunk.collapsed {
            for row in &hunk.rows {
                if row
                    .old_text
                    .as_deref()
                    .unwrap_or_default()
                    .to_lowercase()
                    .contains(&query)
                    || row
                        .new_text
                        .as_deref()
                        .unwrap_or_default()
                        .to_lowercase()
                        .contains(&query)
                {
                    matches.push(row_index);
                }
                row_index += 1;
            }
        }
    }
    matches
}
