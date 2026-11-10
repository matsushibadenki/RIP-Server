//! Bounded, cached inventory of the actual sandbox images, not host fonts.
use super::*;
use std::{process::Stdio, time::Duration};
use tokio::io::AsyncReadExt;
static CACHE: tokio::sync::Mutex<Option<(i64, serde_json::Value)>> =
    tokio::sync::Mutex::const_new(None);
const GS_QUERY: &str = r"(*) { { dup findfont /FontName get exch (JRIP_FONT ) print 256 string cvs print (\t) print = } stopped {clear} if } 256 string /Font resourceforall quit";
const MUPDF_QUERY: &str = r#"
import sys,json,pathlib,subprocess
sys.path.append('/usr/local/lib/jrip')
from font_policy import STANDARD14
names=sorted(STANDARD14)
stream='\n'.join('BT /F%d 5 Tf 2 8 Td (A) Tj ET'%i for i in range(len(names))).encode()
resources=' '.join('/F%d %d 0 R'%(i,i+5) for i in range(len(names)))
objects=[b'<< /Type /Catalog /Pages 2 0 R >>',b'<< /Type /Pages /Kids [3 0 R] /Count 1 >>',('<< /Type /Page /Parent 2 0 R /MediaBox [0 0 20 20] /Resources << /Font << '+resources+' >> >> /Contents 4 0 R >>').encode(),b'<< /Length '+str(len(stream)).encode()+b' >>\nstream\n'+stream+b'\nendstream']
objects += [('<< /Type /Font /Subtype /Type1 /BaseFont /'+name+' >>').encode() for name in names]
data=bytearray(b'%PDF-1.4\n');offsets=[0]
for i,obj in enumerate(objects,1):
 offsets.append(len(data));data+=('%d 0 obj\n'%i).encode()+obj+b'\nendobj\n'
xref=len(data);data+=('xref\n0 %d\n0000000000 65535 f \n'%len(offsets)).encode()
for n in offsets[1:]:data+=('%010d 00000 n \n'%n).encode()
data+=('trailer\n<< /Size %d /Root 1 0 R >>\nstartxref\n%d\n%%%%EOF\n'%(len(offsets),xref)).encode()
pathlib.Path('/tmp/font-probe.pdf').write_bytes(data)
subprocess.run(['mutool','draw','-q','-F','pam','-c','rgb','-r','72','-o','/tmp/font-probe.pam','/tmp/font-probe.pdf'],check=True,timeout=10,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
print(json.dumps(names))
"#;
async fn output(args: &[String], limit: u64) -> Result<Vec<u8>, ()> {
    tokio::time::timeout(Duration::from_secs(20), async {
        let mut child = tokio::process::Command::new("docker")
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .map_err(|_| ())?;
        let mut bytes = Vec::new();
        child
            .stdout
            .take()
            .ok_or(())?
            .take(limit + 1)
            .read_to_end(&mut bytes)
            .await
            .map_err(|_| ())?;
        if bytes.len() as u64 > limit {
            return Err(());
        }
        if !child.wait().await.map_err(|_| ())?.success() {
            return Err(());
        }
        Ok(bytes)
    })
    .await
    .map_err(|_| ())?
}
fn name(value: &str) -> bool {
    !value.is_empty() && value.len() <= 256 && value.bytes().all(|b| (33..=126).contains(&b))
}
fn gs_fonts(bytes: &[u8]) -> Result<Vec<serde_json::Value>, ()> {
    let text = std::str::from_utf8(bytes).map_err(|_| ())?;
    let mut fonts = std::collections::BTreeMap::new();
    for line in text.lines() {
        if let Some(line) = line.strip_prefix("JRIP_FONT ") {
            let (font, resolved) = line.split_once('\t').ok_or(())?;
            if !name(font) || !name(resolved) {
                return Err(());
            }
            fonts.insert(font, resolved);
        }
    }
    if fonts.is_empty() || fonts.len() > 512 {
        return Err(());
    }
    Ok(fonts.into_iter().map(|(font,resolved)|serde_json::json!({"name":font,"resolvedName":resolved,"engine":"ghostscript","formats":if STANDARD14.contains(&font){vec!["PDF","PS","EPS"]}else{vec!["PS","EPS"]},"pdfSubstitution":STANDARD14.contains(&font)})).collect())
}
const STANDARD14: [&str; 14] = [
    "Times-Roman",
    "Times-Bold",
    "Times-Italic",
    "Times-BoldItalic",
    "Helvetica",
    "Helvetica-Bold",
    "Helvetica-Oblique",
    "Helvetica-BoldOblique",
    "Courier",
    "Courier-Bold",
    "Courier-Oblique",
    "Courier-BoldOblique",
    "Symbol",
    "ZapfDingbats",
];
pub(crate) async fn engine(engine: &str, image: &str) -> serde_json::Value {
    let result=async {
        let id=output(&["image".into(),"inspect".into(),"--format".into(),"{{.Id}}".into(),image.into()],1024).await?;
        let id=std::str::from_utf8(&id).map_err(|_|())?.trim();if id.len()!=71||!id.starts_with("sha256:")||!id[7..].bytes().all(|b|b.is_ascii_hexdigit()){return Err(());}
        let container=format!("jrip-font-catalog-{}",Uuid::new_v4());
        let entry=if engine=="mupdf"{"python3"}else{"gs"};
        let mut args:Vec<String>=["run","--rm","--pull=never","--network=none","--read-only","--cap-drop=ALL","--security-opt=no-new-privileges","--memory=128m","--memory-swap=128m","--cpus=0.5","--pids-limit=64","--tmpfs=/tmp:rw,noexec,nosuid,size=16777216","--name",&container,"--entrypoint",entry,id].into_iter().map(String::from).collect();
        if engine=="mupdf"{args.extend(["-I","-c",MUPDF_QUERY].into_iter().map(String::from));}
        else{args.extend(["-q","-dSAFER","-dNODISPLAY","-dNOFONTPATH","-dNOPLATFONTS","-dNONATIVEFONTMAP","-sSUBSTFONT=JRIP-Missing-Font-Rejected","-I/usr/local/lib/jrip/strict-resources","-c",GS_QUERY].into_iter().map(String::from));}
        let bytes=output(&args,128*1024).await;
        let _=tokio::time::timeout(Duration::from_secs(3),tokio::process::Command::new("docker").args(["rm","-f",&container]).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).kill_on_drop(true).status()).await;
        let bytes=bytes?;
        let fonts=if engine=="mupdf"{
            let names:Vec<String>=serde_json::from_slice(&bytes).map_err(|_|())?;
            if names.len()!=14||!STANDARD14.iter().all(|n|names.iter().any(|v|v==n)){return Err(());}
            names.iter().map(|n|serde_json::json!({"name":n,"resolvedName":null,"engine":"mupdf","formats":["PDF"],"pdfSubstitution":true})).collect()
        }else{gs_fonts(&bytes)?};
        Ok::<_,()>(serde_json::json!({"engine":engine,"status":"available","imageId":id,"fonts":fonts}))
    }.await;
    result.unwrap_or_else(|_|serde_json::json!({"engine":engine,"status":"unavailable","reasonCode":"font_catalog_unavailable","fonts":[]}))
}
pub(crate) async fn get(
    State(app): State<App>,
    headers: HeaderMap,
) -> Result<axum::response::Response, ApiError> {
    authorize(&app, &headers, "jobs:read")?;
    let mut cache = CACHE
        .try_lock()
        .map_err(|_| ApiError(StatusCode::SERVICE_UNAVAILABLE, "font_catalog_busy"))?;
    if let Some((expires, value)) = &*cache
        && *expires > rip_storage::now()
    {
        let value = value.clone();
        drop(cache);
        return registered(&app, value).await;
    }
    // Keep cleanup and single-flight ownership alive even when the HTTP client leaves.
    let value=tokio::spawn(async move {
        let mupdf=engine("mupdf","jrip-mupdf:local").await;let ghostscript=engine("ghostscript","jrip-ghostscript:local").await;
        let now=rip_storage::now();let value=serde_json::json!({"profile":"org.jrip.font-catalog-v1","observedAt":now,"expiresAt":now+30,"engines":[mupdf,ghostscript],"hostFontsIncluded":false,"embeddedFontsRequireDocument":true});
        *cache=Some((now+30,value.clone()));value
    }).await.map_err(internal)?;
    registered(&app, value).await
}
async fn registered(
    app: &App,
    mut value: serde_json::Value,
) -> Result<axum::response::Response, ApiError> {
    if let Some(snapshot) = crate::font_install::catalog(app.root.clone()).await? {
        value["registeredSnapshotSha256"] = serde_json::json!(snapshot.digest);
        if let Some(engines) = value["engines"].as_array_mut() {
            for engine in engines {
                let name = engine["engine"].as_str().unwrap_or("").to_owned();
                if let Some(fonts) = engine["fonts"].as_array_mut() {
                    for f in &snapshot.fonts {
                        fonts.push(serde_json::json!({"name":f.name,"resolvedName":f.name,"engine":name,"formats":["PDF","PS","EPS"],"pdfSubstitution":false,"source":"installed","sha256":f.sha256,"family":f.family,"style":f.style}));
                    }
                }
            }
        }
    }
    Ok(reply(value))
}
fn reply(value: serde_json::Value) -> axum::response::Response {
    use axum::response::IntoResponse;
    (
        [(axum::http::header::CACHE_CONTROL, "private, no-store")],
        Json(value),
    )
        .into_response()
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn catalogs_are_sorted_and_pdf_names_are_policy_limited() {
        let fonts=gs_fonts(b"notice\nJRIP_FONT NimbusSans-Regular\tNimbusSans-Regular\nJRIP_FONT Helvetica\tNimbusSans-Regular\n").unwrap();
        assert_eq!(fonts[0]["name"], "Helvetica");
        assert_eq!(fonts[0]["formats"], serde_json::json!(["PDF", "PS", "EPS"]));
        assert_eq!(fonts[1]["formats"], serde_json::json!(["PS", "EPS"]));
        assert!(gs_fonts(b"JRIP_FONT bad name\tface\n").is_err());
        assert!(gs_fonts(b"diagnostic only\n").is_err());
    }
    #[tokio::test]
    async fn inventory_requires_authorization_before_docker() {
        let dir = tempfile::tempdir().unwrap();
        let app = crate::open(dir.path().into(), "token".into())
            .await
            .unwrap();
        assert!(matches!(
            get(State(app), HeaderMap::new()).await,
            Err(ApiError(StatusCode::UNAUTHORIZED, _))
        ));
    }
}

#[cfg(test)]
mod docker_tests {
    use super::*;
    #[tokio::test]
    #[ignore = "requires actual jrip-mupdf:local and jrip-ghostscript:local images"]
    async fn actual_engine_fonts_resolve_under_worker_restrictions() {
        let m = engine("mupdf", "jrip-mupdf:local").await;
        let g = engine("ghostscript", "jrip-ghostscript:local").await;
        assert_eq!(m["status"], "available", "{m}");
        assert_eq!(m["fonts"].as_array().unwrap().len(), 14);
        assert_eq!(g["status"], "available", "{g}");
        assert!(
            g["fonts"]
                .as_array()
                .unwrap()
                .iter()
                .any(|f| f["name"] == "Helvetica")
        );
        assert!(
            g["fonts"]
                .as_array()
                .unwrap()
                .iter()
                .any(|f| f["name"] == "NimbusSans-Regular"
                    && f["formats"] == serde_json::json!(["PS", "EPS"]))
        );
    }
}
