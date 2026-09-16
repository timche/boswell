const NAMED: usize = 3;

pub fn commit_subject(files: &[String]) -> String {
    if files.len() <= NAMED {
        format!("Update {}", files.join(", "))
    } else {
        format!(
            "Update {} and {} more",
            files[..NAMED].join(", "),
            files.len() - NAMED
        )
    }
}

#[cfg(test)]
mod tests {
    use super::commit_subject;

    fn names(n: usize) -> Vec<String> {
        (0..n).map(|i| format!("f{i}.md")).collect()
    }

    #[test]
    fn one_file() {
        assert_eq!(commit_subject(&names(1)), "Update f0.md");
    }

    #[test]
    fn two_files() {
        assert_eq!(commit_subject(&names(2)), "Update f0.md, f1.md");
    }

    #[test]
    fn three_files() {
        assert_eq!(commit_subject(&names(3)), "Update f0.md, f1.md, f2.md");
    }

    #[test]
    fn four_files() {
        assert_eq!(
            commit_subject(&names(4)),
            "Update f0.md, f1.md, f2.md and 1 more"
        );
    }

    #[test]
    fn ten_files() {
        assert_eq!(
            commit_subject(&names(10)),
            "Update f0.md, f1.md, f2.md and 7 more"
        );
    }

    #[test]
    fn paths_are_not_capitalised_or_reordered() {
        let files = vec!["docs/z.md".to_string(), "a.md".to_string()];
        assert_eq!(commit_subject(&files), "Update docs/z.md, a.md");
    }
}
