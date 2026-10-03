use serde::Serialize;
use serde_json::{Value, json};

pub const ACX_VERSION: &str = "0.1";
pub const RIP_CAPABILITY_ID: &str = "org.jrip.rip.export_tiff";
pub mod intent;

#[derive(Debug, Clone, Serialize)]
pub struct Manifest {
    pub acx: &'static str,
    pub provider: Provider,
    pub capabilities: Vec<Capability>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Provider {
    pub id: &'static str,
    pub name: &'static str,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Capability {
    pub id: &'static str,
    pub version: &'static str,
    pub description: &'static str,
    pub input_schema: Value,
    pub output_schema: Value,
    pub bindings: Vec<Binding>,
    pub risk: &'static str,
    pub authority: Authority,
    pub effects: Vec<&'static str>,
    pub economics: Economics,
    pub evidence: Evidence,
    pub recovery: Recovery,
    pub extensions: Value,
}

#[derive(Debug, Clone, Serialize)]
pub struct Binding {
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub profile: &'static str,
    pub endpoint: &'static str,
}

#[derive(Debug, Clone, Serialize)]
pub struct Authority {
    pub approval: &'static str,
    pub scopes: Vec<&'static str>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Economics {
    pub model: &'static str,
}

#[derive(Debug, Clone, Serialize)]
pub struct Evidence {
    pub receipt: &'static str,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Recovery {
    pub reversible: bool,
    pub cancel_until: &'static str,
}

pub fn manifest() -> Manifest {
    Manifest {
        acx: ACX_VERSION,
        provider: Provider {
            id: "urn:acx:provider:jrip-local",
            name: "J-RIP Server",
        },
        capabilities: vec![Capability {
            id: RIP_CAPABILITY_ID,
            version: "0.1.0",
            description: "Validate and render a PDF, PostScript, or EPS document to TIFF files.",
            input_schema: print_intent_schema(),
            output_schema: json!({
                "type": "object",
                "required": ["jobId", "state"],
                "properties": {
                    "jobId": {"type": "string", "format": "uuid"},
                    "state": {"type": "string"},
                    "selectedEngine": {"enum": ["mupdf", "ghostscript"]}
                },
                "additionalProperties": false
            }),
            bindings: vec![Binding {
                kind: "https",
                profile: "org.jrip.multipart-job-v1",
                endpoint: "/api/v1/jobs",
            }],
            risk: "reversible",
            authority: Authority {
                approval: "policy",
                scopes: vec!["jobs:submit"],
            },
            effects: vec!["compute", "persistent-storage", "raster-output"],
            economics: Economics { model: "external" },
            evidence: Evidence { receipt: "plain" },
            recovery: Recovery {
                reversible: true,
                cancel_until: "terminal",
            },
            extensions: json!({
                "org.jrip.print": {
                    "stage": "rip",
                    "resolveEndpoint": "/api/v1/print/resolve",
                    "resolveProfile": "acx-print-intent-0.1-experimental",
                    "resolutionOnly": true,
                    "artifactDownloadTemplate": "/api/v1/jobs/{id}/artifacts/{name}",
                    "artifactVerifyTemplate": "/api/v1/jobs/{id}/artifacts/{name}/verify",
                    "engines": ["mupdf", "ghostscript"],
                    "documentFormats": [
                        "application/pdf",
                        "application/postscript",
                        "application/eps"
                    ],
                    "outputFormats": ["image/tiff"],
                    "dpi": {"minimum": 72, "maximum": 2400, "default": 600},
                    "colorModes": ["rgb", "cmyk", "gray"],
                    "physicalOutput": false
                }
            }),
        }],
    }
}

fn print_intent_schema() -> Value {
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "required": ["document", "name", "format"],
        "properties": {
            "document": {"type": "string", "format": "uri"},
            "name": {"type": "string", "minLength": 1, "maxLength": 255},
            "format": {"enum": [
                "application/pdf",
                "application/postscript",
                "application/eps"
            ]},
            "dpi": {"type": "integer", "minimum": 72, "maximum": 2400, "default": 600},
            "colorMode": {"enum": ["rgb", "cmyk", "gray"], "default": "cmyk"},
            "engine": {"enum": ["auto", "mupdf", "ghostscript"], "default": "auto"},
            "priority": {"type": "integer", "minimum": 0, "maximum": 100, "default": 50}
        },
        "additionalProperties": false
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn advertises_only_the_current_file_output_boundary() {
        let value = serde_json::to_value(manifest()).unwrap();
        let capability = &value["capabilities"][0];
        assert_eq!(capability["id"], RIP_CAPABILITY_ID);
        assert_eq!(
            capability["extensions"]["org.jrip.print"]["physicalOutput"],
            false
        );
        assert_eq!(capability["bindings"][0]["endpoint"], "/api/v1/jobs");
        assert!(
            capability["bindings"]
                .as_array()
                .unwrap()
                .iter()
                .all(|binding| binding["type"] != "mcp")
        );
    }
}
