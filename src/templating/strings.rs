use sailfish::TemplateOnce;

#[derive(TemplateOnce)]
#[template(path = "strings.xml", delimiter = '#')]
pub struct StringsRes {
    pub app_name: String,
    pub main_activity_title: String,
}

impl StringsRes {
    pub fn new(app_name: &str, main_activity_title: &str) -> StringsRes {
        StringsRes {
            app_name: String::from(app_name),
            main_activity_title: String::from(main_activity_title),
        }
    }
}
