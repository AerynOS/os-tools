// SPDX-FileCopyrightText: Copyright © 2020-2024 Serpent OS Developers
//
// SPDX-License-Identifier: MPL-2.0

use std::io;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use thiserror::Error;
use tui::Styled;

#[derive(Serialize)]
struct MonitoringTemplate {
    releases: Releases,
    security: Security,
}

#[derive(Serialize)]
struct Releases {
    id: Option<u32>,
    rss: Option<String>,
}

#[derive(Serialize)]
struct Security {
    cpe: Vec<Cpe>,
}

#[derive(Serialize)]
struct Cpe {
    vendor: Option<String>,
    product: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Item {
    id: u32,
    name: String,
}

#[derive(Debug, Deserialize)]
struct Response {
    items: Vec<Item>,
    total_items: u32,
}

#[derive(Serialize)]
pub struct Monitoring<'a> {
    name: &'a String,
    homepage: &'a String,
}

impl<'a> Monitoring<'a> {
    pub fn new(name: &'a String, homepage: &'a String) -> Self {
        Self { name, homepage }
    }

    pub fn run(&self) -> Result<String, Error> {
        if self.name.is_empty() {
            return self.format_monitoring(None, vec![], None);
        }

        let client = reqwest::blocking::Client::new();

        let id = self.find_monitoring_id(self.name, &client)?;
        let cpes = self.find_security_cpe(self.name, &client)?;
        let rss = self.guess_rss(self.homepage, self.name);

        let output = self.format_monitoring(id, cpes, rss)?;

        Ok(output)
    }

    fn find_monitoring_id(&self, name: &String, client: &reqwest::blocking::Client) -> Result<Option<u32>, Error> {
        let url = format!("https://release-monitoring.org/api/v2/projects/?name={name}");

        let resp = client.get(&url).send()?;

        match resp.error_for_status_ref() {
            Ok(_res) => (),
            Err(err) => return Err(Error::StatusCode(err)),
        }

        let body: Response = resp.json()?;

        if body.total_items == 1 {
            if let Some(result) = body.items.first() {
                println!(
                    "{} | Matched id {} from {}",
                    "Monitoring".green(),
                    result.id,
                    result.name
                );
                Ok(Some(result.id))
            } else {
                Ok(None)
            }
        } else if body.total_items > 1 && body.total_items < 10 {
            println!(
                "{} | Multiple potential IDs matched, find the correct ID for the project at https://release-monitoring.org/",
                "Warning".yellow()
            );
            for i in body.items {
                println!(
                    "ID {} Name {} URL https://release-monitoring.org/project/{}/",
                    i.id, i.name, i.id
                );
            }
            println!();
            Ok(None)
        } else {
            println!(
                "{} | Find the correct ID for the project at https://release-monitoring.org/",
                "Warning".yellow()
            );
            Ok(None)
        }
    }

    fn find_security_cpe(&self, name: &String, client: &reqwest::blocking::Client) -> Result<Vec<Cpe>, Error> {
        const URL: &str = "https://cpe-guesser.cve-search.org/search";

        let mut query = HashMap::new();
        query.insert("query", [name]);

        let resp = client.post(URL).json(&query).send()?;

        match resp.error_for_status_ref() {
            Ok(_res) => (),
            Err(err) => return Err(Error::StatusCode(err)),
        }

        let json: Vec<Vec<Value>> = serde_json::from_str(&resp.text()?).unwrap_or_default();

        // Extract CPEs into a Vec<CPE>
        let cpes: Vec<Cpe> = json
            .iter()
            .map(|item| {
                if let Some(Value::String(cpe_string)) = item.get(1) {
                    // Split the CPE string and extract the desired parts
                    let parts: Vec<&str> = cpe_string.split(':').collect();
                    if parts.len() > 4 {
                        let vendor = parts[3].to_owned();
                        let product = parts[4].to_owned();
                        println!(
                            "{} | Matched CPE Vendor: {vendor} Product: {product}",
                            "Security".green()
                        );
                        return Cpe {
                            vendor: Some(vendor),
                            product: Some(product),
                        };
                    }
                }
                Cpe {
                    vendor: None,
                    product: None,
                }
            })
            .collect();
        println!();

        if cpes.len() > 1 {
            println!(
                "{} | Multiple CPEs matched, please verify and remove any superfluous",
                "Warning".yellow()
            );
        }

        Ok(cpes)
    }

    fn guess_rss(&self, homepage: &String, name: &String) -> Option<String> {
        match homepage {
            _ if homepage.starts_with("https://github.com") => Some(format!("{homepage}/releases.atom")),
            _ if homepage.starts_with("https://files.pythonhosted.org")
                || homepage.starts_with("https://pypi.org")
                || homepage.starts_with("https://pypi.python.org")
                || homepage.starts_with("https://pypi.io") =>
            {
                Some(format!("https://pypi.org/rss/project/{name}/releases.xml"))
            }
            _ => None,
        }
    }

    fn format_monitoring(&self, id: Option<u32>, cpes: Vec<Cpe>, rss: Option<String>) -> Result<String, Error> {
        let monitoring_template = MonitoringTemplate {
            releases: Releases {
                id: Some(id.unwrap_or_default()),
                rss: Some(rss.unwrap_or_default()),
            },
            security: Security { cpe: cpes },
        };

        let mut yaml_string = serde_yaml::to_string(&monitoring_template).expect("Failed to serialize to YAML");

        // We may not have matched any ID or CPE which is fine
        // Unwrap the default value then mangle it into a YAML ~ (null) value
        if monitoring_template.releases.id.unwrap_or_default() == 0 {
            let id_string = "id: 0";
            let id_marker = yaml_string.find(id_string).expect("releases id marker not found");
            yaml_string = yaml_string.replace(id_string, "id: ~");
            const ID_HELP_TEXT: &str =
                " # https://release-monitoring.org/ and use the numeric id in the url of project";
            yaml_string.insert_str(id_marker + id_string.len(), ID_HELP_TEXT);
        }

        if monitoring_template.releases.rss.unwrap_or_default().is_empty() {
            yaml_string = yaml_string.replace("rss: ''", "rss: ~");
        }

        if monitoring_template.security.cpe.is_empty() {
            let cpe_string = "cpe: []";
            let cpe_marker = yaml_string.find(cpe_string).expect("security cpe marker not found");
            yaml_string = yaml_string.replace(cpe_string, "cpe: ~");
            let cpe_help_text = format!(
                " # Last checked {}",
                chrono::Local::now().date_naive().format("%Y-%m-%d")
            );
            yaml_string.insert_str(cpe_marker + cpe_string.len() - 1, &cpe_help_text);
        }

        Ok(yaml_string)
    }
}

#[derive(Debug, Error)]
pub enum Error {
    #[error("io")]
    Io(#[from] io::Error),
    #[error("statuscode")]
    StatusCode(#[from] reqwest::Error),
}
