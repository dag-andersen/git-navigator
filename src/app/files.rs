use std::{
    collections::BTreeMap,
    ffi::OsString,
    path::{Component, Path, PathBuf},
};

use crate::model::{ChangedFile, FileTreeRow};

use super::moved_selection;

pub(crate) fn first_file_row(tree: &[FileTreeRow]) -> Option<usize> {
    tree.iter().position(|row| !row.is_directory())
}

pub(crate) fn first_file_row_in(tree: &[FileTreeRow], visible: &[usize]) -> Option<usize> {
    visible
        .iter()
        .copied()
        .find(|index| tree[*index].file_index.is_some())
}

pub(crate) fn moved_visible_file_selection(
    selected: Option<usize>,
    visible: &[usize],
    tree: &[FileTreeRow],
    delta: isize,
) -> Option<usize> {
    let file_positions: Vec<usize> = visible
        .iter()
        .enumerate()
        .filter_map(|(position, row)| (!tree[*row].is_directory()).then_some(position))
        .collect();
    let current = selected
        .and_then(|selected| {
            file_positions
                .iter()
                .position(|position| visible[*position] == selected)
        })
        .or_else(|| (!file_positions.is_empty()).then_some(0));
    let next = moved_selection(current, file_positions.len(), delta)?;
    Some(visible[file_positions[next]])
}

#[derive(Default)]
struct Directory {
    children: BTreeMap<OsString, Node>,
}

enum Node {
    Directory(Directory),
    File(usize),
}

pub(crate) fn build_tree(files: &[ChangedFile]) -> Vec<FileTreeRow> {
    let mut root = Directory::default();
    for (file_index, file) in files.iter().enumerate() {
        let components: Vec<OsString> = file
            .path
            .components()
            .filter_map(|component| match component {
                Component::Normal(name) => Some(name.to_os_string()),
                _ => None,
            })
            .collect();
        insert(&mut root, &components, file_index);
    }

    let mut rows = Vec::new();
    flatten(&root, Path::new(""), "", &mut rows);
    rows
}

fn insert(directory: &mut Directory, components: &[OsString], file_index: usize) {
    let Some((name, remainder)) = components.split_first() else {
        return;
    };
    if remainder.is_empty() {
        directory
            .children
            .insert(name.clone(), Node::File(file_index));
        return;
    }

    let node = directory
        .children
        .entry(name.clone())
        .or_insert_with(|| Node::Directory(Directory::default()));
    if let Node::Directory(child) = node {
        insert(child, remainder, file_index);
    }
}

fn flatten(
    directory: &Directory,
    path_prefix: &Path,
    label_prefix: &str,
    rows: &mut Vec<FileTreeRow>,
) {
    let child_count = directory.children.len();
    for (position, (name, node)) in directory.children.iter().enumerate() {
        let is_last = position + 1 == child_count;
        let path = path_prefix.join(name);
        rows.push(FileTreeRow {
            label: format!(
                "{label_prefix}{}{}",
                if is_last { "└── " } else { "├── " },
                name.to_string_lossy(),
            ),
            path: path.clone(),
            file_index: match node {
                Node::Directory(_) => None,
                Node::File(index) => Some(*index),
            },
        });

        if let Node::Directory(child) = node {
            flatten(
                child,
                &path,
                &format!("{label_prefix}{}", if is_last { "    " } else { "│   " }),
                rows,
            );
        }
    }
}

pub(crate) fn filtered_tree(
    tree: &[FileTreeRow],
    files: &[ChangedFile],
    query: &str,
) -> Vec<usize> {
    if query.is_empty() {
        return (0..tree.len()).collect();
    }

    let matching_files: Vec<usize> = tree
        .iter()
        .enumerate()
        .filter(|(_, row)| {
            row.file_index.is_some_and(|file_index| {
                files
                    .get(file_index)
                    .is_some_and(|file| super::fuzzy_match(query, &file.path.display().to_string()))
            })
        })
        .map(|(row_index, _)| row_index)
        .collect();

    tree.iter()
        .enumerate()
        .filter(|(row_index, row)| {
            row.file_index
                .is_some_and(|_| matching_files.contains(row_index))
                || row.file_index.is_none()
                    && matching_files
                        .iter()
                        .any(|file_index| tree[*file_index].path.starts_with(&row.path))
        })
        .map(|(row_index, _)| row_index)
        .collect()
}

pub(crate) fn tree_label(tree: &[FileTreeRow], row_index: usize, visible_rows: &[usize]) -> String {
    let path = &tree[row_index].path;
    let depth = path.components().count();
    let mut label = String::new();
    for ancestor_depth in 1..depth {
        let ancestor =
            path.components()
                .take(ancestor_depth)
                .fold(PathBuf::new(), |mut path, component| {
                    path.push(component.as_os_str());
                    path
                });
        let has_later_sibling = visible_rows.iter().any(|index| {
            tree[*index].path.parent() == ancestor.parent()
                && tree[*index].path != ancestor
                && *index
                    > tree
                        .iter()
                        .position(|row| row.path == ancestor)
                        .unwrap_or(0)
        });
        label.push_str(if has_later_sibling { "│   " } else { "    " });
    }
    let has_later_sibling = visible_rows.iter().any(|index| {
        *index > row_index
            && tree[*index].path.parent() == path.parent()
            && tree[*index].path != *path
    });
    label.push_str(if has_later_sibling {
        "├── "
    } else {
        "└── "
    });
    label.push_str(&path.file_name().map_or_else(
        || path.display().to_string(),
        |name| name.to_string_lossy().into_owned(),
    ));
    label
}
