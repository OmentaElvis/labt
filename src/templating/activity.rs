use sailfish::TemplateOnce;

#[derive(TemplateOnce)]
#[template(path = "Activity.java", delimiter = '#')]
pub struct Activity {
    pub package_name: String,
    pub class_name: String,
    pub xml_layout: Option<String>,
}

impl Activity {
    pub fn new(package_name: &str, class_name: &str, xml_layout: Option<String>) -> Activity {
        return Activity {
            package_name: String::from(package_name),
            class_name: String::from(class_name),
            xml_layout,
        };
    }
}
