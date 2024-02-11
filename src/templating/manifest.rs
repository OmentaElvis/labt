use sailfish::TemplateOnce;

#[derive(TemplateOnce)]
#[template(path = "AndroidManifest.xml", delimiter = '#')]
pub struct AndroidManifest {
    pub package_name: String,
    pub version_number: i32,
    pub version_name: String,
    pub main_activity: String,
}

impl AndroidManifest {
    pub fn new(
        package: &str,
        version_number: i32,
        version_name: &str,
        main_activity: &str,
    ) -> AndroidManifest {
        AndroidManifest {
            package_name: String::from(package),
            version_number,
            version_name: String::from(version_name),
            main_activity: String::from(main_activity),
        }
    }
}
