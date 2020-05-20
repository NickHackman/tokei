// comment
/*

comment line

*/

#[cfg(test)]
mod annotate {
    use tokei::{Config, LanguageType, LineType};

    #[test]
    fn annotate_this_file() {
        let file = file!();
        let config = Config::default();
        let lang_type = LanguageType::from_path(&file, &config).unwrap();
        let annotated = lang_type.annotate_file(file, &config).unwrap();
        assert_eq!(annotated.len(), 25);
        assert_eq!(annotated.get(&1).unwrap(), &LineType::Comment);
        assert_eq!(annotated.get(&4).unwrap(), &LineType::Comment);
        assert_eq!(annotated.get(&8).unwrap(), &LineType::Code);
        assert_eq!(annotated.get(&3).unwrap(), &LineType::Blank);
        assert_eq!(annotated.get(&5).unwrap(), &LineType::Blank);
    }
}
