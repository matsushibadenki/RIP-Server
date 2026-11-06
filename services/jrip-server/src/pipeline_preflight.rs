//! Live physical PrintIntent negotiation followed by bound, atomic source acceptance.
use super::*;
use rip_acx::intent::{Destination, Mode, PrintIntent, Requirement};
use serde::Serialize;
use serde_json::{Value, json};

const PROFILE: &str = "org.jrip.pipeline-preflight-v2";
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Request {
    request_id: Uuid,
    intent: PrintIntent,
    color: Option<pipeline::ColorRequest>,
    inspection_id: Option<Uuid>,
}
fn select<T: Clone + Serialize + PartialEq>(
    field: &str,
    requirement: Option<Requirement<T>>,
    options: &[T],
    default: T,
    conflicts: &mut Vec<Value>,
    adjustments: &mut Vec<Value>,
) -> T {
    let Some(r) = requirement else {
        return default;
    };
    if r.mode != Mode::Automatic && options.contains(&r.value) {
        return r.value;
    }
    if r.mode == Mode::Required {
        conflicts.push(json!({"field":field,"code":"required_setting_unavailable","requested":r.value,"alternatives":options,"retryable":false}));
    } else if r.value != default {
        adjustments.push(json!({"field":field,"requested":r.value,"resolved":default,"reasonCode":if r.mode==Mode::Automatic {"AUTOMATIC_SELECTION"}else{"PREFERENCE_NOT_SUPPORTED"},"alternatives":options}));
    }
    default
}
pub async fn create(
    State(app): State<App>,
    headers: HeaderMap,
    Json(mut req): Json<Request>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    pipeline::scopes(&app, &headers)?;
    let digest_profile = preflight::selected_digest(&headers)?;
    if req.intent.output.destination != Destination::Printer {
        return Err(bad("printer_destination_required"));
    }
    let printer_id = req
        .intent
        .output
        .printer_id
        .clone()
        .ok_or(bad("printer_not_configured"))?;
    if req
        .intent
        .idempotency_key
        .as_ref()
        .is_some_and(|key| key != &req.request_id.to_string())
    {
        return Err(bad("idempotency_key_mismatch"));
    }
    let snapshot = print::models(&app, Some(&printer_id)).await?;
    let intersection = &snapshot["deliveryIntersection"];
    let mut conflicts = intersection["conflicts"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let mut adjustments = vec![];
    let side_options: Vec<String> =
        serde_json::from_value(intersection["deliverySides"].clone()).map_err(internal)?;
    let sides: rip_core::print::Sides = serde_json::from_value(json!(select(
        "sides",
        Some(req.intent.sides.take().unwrap_or(Requirement {
            mode: Mode::Required,
            value: "one-sided".into()
        })),
        &side_options,
        side_options.first().cloned().unwrap_or("one-sided".into()),
        &mut conflicts,
        &mut adjustments
    )))
    .map_err(internal)?;
    // Preserve original page numbers through rendering and delivery.
    let page_ranges = if let Some(requirement) = req.intent.page_ranges.take() {
        match rip_core::pages::PageRanges::new(
            requirement
                .value
                .iter()
                .map(|range| rip_core::pages::PageRange {
                    first: range.first,
                    last: range.last,
                })
                .collect(),
        ) {
            Ok(ranges) if requirement.mode != Mode::Automatic => ranges,
            Ok(ranges) => {
                if !ranges.is_all() {
                    adjustments.push(json!({"field":"pageRanges","requested":ranges,"resolved":[],"reasonCode":"AUTOMATIC_SELECTION","alternatives":[[]]}));
                }
                Default::default()
            }
            Err(_) => {
                conflicts.push(json!({"field":"pageRanges","code":"invalid_page_ranges","requested":requirement.value,"alternatives":[],"retryable":false}));
                Default::default()
            }
        }
    } else {
        Default::default()
    };
    // Physical copies repeat the delivered document, never the source rasterization.
    let copy_options: Vec<u32> =
        serde_json::from_value::<Option<[u32; 2]>>(intersection["deliveryCopies"].clone())
            .map_err(internal)?
            .map(|[min, max]| (min..=max).collect())
            .unwrap_or_default();
    if !(1..=rip_core::print::MAX_DELIVERY_COPIES).contains(&req.intent.copies.value) {
        conflicts.push(json!({"field":"copies","code":"invalid_copies","requested":req.intent.copies.value,"alternatives":copy_options,"retryable":false}));
    }
    let copies = select(
        "copies",
        Some(req.intent.copies.clone()),
        &copy_options,
        copy_options.first().copied().unwrap_or(1),
        &mut conflicts,
        &mut adjustments,
    );
    req.intent.copies = Requirement {
        mode: Mode::Required,
        value: 1,
    };
    let dpis: Vec<u32> = intersection["dpi"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|v| v[0].as_u64().map(|n| n as u32))
        .collect();
    let colors: Vec<String> = if let Some(color) = &req.color {
        let input = serde_json::to_value(color.policy.input_color)
            .map_err(internal)?
            .as_str()
            .unwrap()
            .to_string();
        let output = serde_json::to_value(color.policy.output_color).map_err(internal)?;
        if !intersection["colorOutputSpaces"]
            .as_array()
            .is_some_and(|values| values.contains(&output))
        {
            conflicts.push(json!({"field":"color.policy.outputColor","code":"pipeline_color_mismatch","alternatives":intersection["colorOutputSpaces"],"retryable":false}));
        }
        vec![input]
    } else {
        intersection["directColorSpaces"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|v| v.as_str().map(str::to_string))
            .collect()
    };
    let media: Vec<String> = intersection["media"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|v| v["name"].as_str().map(str::to_string))
        .collect();
    let dpi = select(
        "dpi",
        req.intent.dpi.take(),
        &dpis,
        if dpis.contains(&600) {
            600
        } else {
            dpis.first().copied().unwrap_or(600)
        },
        &mut conflicts,
        &mut adjustments,
    );
    let color_mode = select(
        "colorMode",
        req.intent.color_mode.take(),
        &colors,
        colors.first().cloned().unwrap_or("rgb".into()),
        &mut conflicts,
        &mut adjustments,
    );
    let selected_media = select(
        "media",
        req.intent.media.take(),
        &media,
        media.first().cloned().unwrap_or_default(),
        &mut conflicts,
        &mut adjustments,
    );
    // The shared resolver validates the remaining raster settings; physical choices are resolved above.
    req.intent.output.destination = Destination::Artifact;
    req.intent.output.printer_id = None;
    req.intent.dpi = Some(Requirement {
        mode: Mode::Required,
        value: dpi,
    });
    req.intent.color_mode = Some(Requirement {
        mode: Mode::Required,
        value: color_mode,
    });
    if req.intent.document.media_type == "application/pdf" {
        req.intent.page_ranges = Some(Requirement {
            mode: Mode::Required,
            value: serde_json::from_value(serde_json::to_value(&page_ranges).map_err(internal)?)
                .map_err(internal)?,
        });
    }
    let resolution = rip_acx::intent::resolve(req.intent);
    conflicts.extend(
        resolution["conflicts"]
            .as_array()
            .cloned()
            .unwrap_or_default(),
    );
    adjustments.extend(
        resolution["adjustments"]
            .as_array()
            .cloned()
            .unwrap_or_default(),
    );
    if !conflicts.is_empty() || resolution["executable"] != true {
        return Ok((
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(
                json!({"executable":false,"conflicts":conflicts,"adjustments":adjustments,"capabilityRevision":snapshot["capabilityRevision"],"authorizationGranted":false,"documentVerified":false,"messages":{"en":"Required settings are incompatible with the current pipeline; inspect conflicts and alternatives","ja":"必須条件が現在の処理経路に合いません。競合と代替案を確認してください","zh-CN":"必要条件与当前流程不兼容，请检查冲突与替代方案"}}),
            ),
        ));
    }
    let manifest: Manifest =
        serde_json::from_value(resolution["ticket"]["manifest"].clone()).map_err(internal)?;
    let source_hash = resolution["ticket"]["document"]["sha256"]
        .as_str()
        .ok_or(bad("invalid_preflight"))?
        .to_string();
    let source = Job {
        id: Uuid::nil(),
        manifest,
        state: JobState::Queued,
        source_sha256: source_hash,
        created_at: 0,
        updated_at: 0,
        revision: 0,
        error_code: None,
        selected_engine: None,
        worker_run_id: None,
    };
    let workflow = pipeline::Request {
        page_ranges,
        sides,
        copies,
        request_id: req.request_id,
        expected_revision: 0,
        printer_id,
        media: selected_media,
        color: req.color,
    };
    let registration = pipeline::registration_ticket(&app, &headers, &source, &workflow)?;
    validate_color(&app, &workflow)?;
    let inspection = if let Some(id) = req.inspection_id {
        Some(validate_inspection(&app, &headers, id, &source, &workflow).await?)
    } else {
        None
    };

    let id = Uuid::new_v4();
    let mut payload = json!({"schemaVersion":1,"profile":if digest_profile==preflight::LEGACY_DIGEST {"org.jrip.pipeline-preflight-v1"}else{PROFILE},"preflightId":id,"capability":"org.jrip.print.pipeline","issuedAt":rip_storage::now(),"expiresAt":snapshot["expiresAt"],"capabilityRevision":snapshot["capabilityRevision"],"ticket":resolution["ticket"],"requestHash":preflight::digest_with_profile(&registration,digest_profile)?,"workflowRequest":workflow,"sourceInspection":inspection,"registrationTicket":registration,"actor":pipeline::actor(&app,&headers)?,"authorizationEvidence":preflight::authorization_evidence(&app,&headers,&["jobs:read","jobs:submit","artifacts:read","printers:print"])?,"adjustments":adjustments,"digestProfile":digest_profile,"authorizationGranted":false,"documentVerified":false,"commitAvailable":true,"effects":if workflow.color.is_some() {json!(["compute","persistent-storage","color-processing","physical-output"])}else{json!(["compute","persistent-storage","physical-output"])},"messages":{"en":"Settings preflight only; geometry, source structure, runtime and approval still require checks","ja":"設定の事前確認です。原稿構造・用紙寸法・実行環境・承認の検査は別途必要です","zh-CN":"仅设置预检，源结构、纸张尺寸、运行环境与审批仍需检查"}});
    payload["preflightDigest"] = json!(preflight::digest_with_profile(&payload, digest_profile)?);
    app.repo
        .save_preflight(
            id,
            &payload,
            snapshot["expiresAt"]
                .as_i64()
                .ok_or(bad("invalid_preflight"))?,
        )
        .await?;
    Ok((StatusCode::CREATED, Json(payload)))
}
pub async fn inspection(
    State(app): State<App>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, ApiError> {
    authorize(&app, &headers, "jobs:submit")?;
    Ok(Json(
        app.repo
            .document_inspection(id, &pipeline::actor(&app, &headers)?)
            .await?,
    ))
}
async fn validate_inspection(
    app: &App,
    headers: &HeaderMap,
    id: Uuid,
    source: &Job,
    workflow: &pipeline::Request,
) -> Result<Value, ApiError> {
    let value = app
        .repo
        .document_inspection(id, &pipeline::actor(app, headers)?)
        .await?;
    if value["sourceSha256"] != source.source_sha256
        || source.manifest.format != rip_core::DocumentFormat::Pdf
    {
        return Err(ApiError(StatusCode::CONFLICT, "inspection_source_mismatch"));
    }
    let printer = app
        .printers
        .iter()
        .find(|printer| printer.id == workflow.printer_id)
        .ok_or(bad("printer_not_configured"))?;
    let media = printer
        .media
        .iter()
        .find(|media| media.name == workflow.media)
        .ok_or(bad("media_not_configured"))?;
    let pages = value["pages"]
        .as_array()
        .filter(|pages| !pages.is_empty() && pages.len() <= 100)
        .ok_or(bad("invalid_inspection"))?;
    if value["schemaVersion"] != 1
        || value["inspectionId"] != id.to_string()
        || value["inspectorRevision"] != "jrip-mupdf-inspection-v1"
        || value["contentParsed"] != true
        || value["repaired"] != false
        || value["encrypted"] != false
    {
        return Err(bad("invalid_inspection"));
    }
    let selected_pages = workflow
        .page_ranges
        .indices(pages.len() as u32)
        .map_err(|_| ApiError(StatusCode::UNPROCESSABLE_ENTITY, "page_range_out_of_bounds"))?;
    for (index, page) in pages.iter().enumerate() {
        if page["page"] != index + 1 {
            return Err(bad("invalid_inspection"));
        }
        for (field, microns) in [
            ("widthPoints", media.width_microns),
            ("heightPoints", media.height_microns),
        ] {
            let points = page[field]
                .as_f64()
                .filter(|points| points.is_finite() && *points > 0.0 && *points <= 2834.6457)
                .ok_or(bad("invalid_inspection"))?;
            let pixels = (points * f64::from(source.manifest.dpi) / 72.0).ceil() as u64;
            let expected = (u64::from(microns) * u64::from(source.manifest.dpi) + 12700) / 25400;
            if selected_pages.contains(&index) && pixels.abs_diff(expected) > 1 {
                return Err(ApiError(
                    StatusCode::UNPROCESSABLE_ENTITY,
                    "inspection_media_mismatch",
                ));
            }
        }
    }
    Ok(value)
}
fn validate_color(app: &App, workflow: &pipeline::Request) -> Result<(), ApiError> {
    if let Some(color) = &workflow.color {
        let registry = rip_color::registry::Registry::open(app.root.join("color-profiles"))
            .map_err(|_| bad("invalid_icc_profile"))?;
        let input = registry
            .read(&color.input_profile_sha256)
            .map_err(|_| bad("invalid_icc_profile"))?;
        let output = color
            .output_profile_sha256
            .as_ref()
            .map(|sha| registry.read(sha))
            .transpose()
            .map_err(|_| bad("invalid_icc_profile"))?;
        // Verify registered profiles and policy using one synthetic pixel; no source rendering or device output.
        rip_color::convert(
            &vec![0; color.policy.input_color.channels()],
            &input,
            output.as_deref(),
            &color.policy,
        )
        .map_err(|_| bad("unsupported_color_policy"))?;
    }
    Ok(())
}
pub async fn commit(
    State(app): State<App>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    mut multipart: Multipart,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    pipeline::scopes(&app, &headers)?;
    let payload = app.repo.get_preflight(id).await?;
    let digest_profile = preflight::stored_digest_profile(&payload, true)?;
    if payload["actor"] != pipeline::actor(&app, &headers)? {
        return Err(ApiError(StatusCode::FORBIDDEN, "preflight_actor_mismatch"));
    }
    let supplied = headers
        .get("x-jrip-preflight-digest")
        .and_then(|h| h.to_str().ok())
        .unwrap_or("");
    let mut unsigned = payload.clone();
    unsigned
        .as_object_mut()
        .ok_or(bad("invalid_preflight"))?
        .remove("preflightDigest");
    if supplied != payload["preflightDigest"].as_str().unwrap_or("")
        || supplied != preflight::digest_with_profile(&unsigned, digest_profile)?
    {
        return Err(ApiError(StatusCode::CONFLICT, "preflight_digest_mismatch"));
    }
    let workflow: pipeline::Request =
        serde_json::from_value(payload["workflowRequest"].clone()).map_err(internal)?;
    if let Some(job) = app.repo.committed_job(id).await? {
        return Ok((
            StatusCode::OK,
            Json(
                json!({"jobId":job,"pipeline":app.repo.pipeline(workflow.request_id).await?,"replayed":true}),
            ),
        ));
    }
    if payload["expiresAt"].as_i64().unwrap_or(0) <= rip_storage::now() {
        return Err(ApiError(StatusCode::GONE, "preflight_expired"));
    }
    let snapshot = print::models(&app, Some(&workflow.printer_id)).await?;
    if snapshot["capabilityRevision"] != payload["capabilityRevision"] {
        return Err(ApiError(
            StatusCode::CONFLICT,
            "capability_revision_changed",
        ));
    }
    let job_id = Uuid::new_v4();
    let directory = app.root.join("jobs").join(job_id.to_string());
    tokio::fs::create_dir(&directory).await.map_err(internal)?;
    let _writer = rip_jobs::upload_guard(&directory).map_err(internal)?;
    let result = async {
        preflight::stage_document(&directory, &mut multipart).await?;
        let manifest =
            serde_json::from_value(payload["ticket"]["manifest"].clone()).map_err(internal)?;
        let expected = payload["ticket"]["document"]["sha256"]
            .as_str()
            .ok_or(bad("invalid_preflight"))?;
        let (job, ticket) = rip_jobs::prepare_staged(&directory, job_id, manifest, Some(expected))
            .await
            .map_err(submit_error)?;
        let registration = pipeline::registration_ticket(&app, &headers, &job, &workflow)?;
        if registration != payload["registrationTicket"] {
            return Err(ApiError(
                StatusCode::CONFLICT,
                "pipeline_configuration_changed",
            ));
        }
        validate_color(&app, &workflow)?;
        if !payload["sourceInspection"].is_null() {
            let id = Uuid::parse_str(
                payload["sourceInspection"]["inspectionId"]
                    .as_str()
                    .ok_or(bad("invalid_inspection"))?,
            )
            .map_err(|_| bad("invalid_inspection"))?;
            if validate_inspection(&app, &headers, id, &job, &workflow).await?
                != payload["sourceInspection"]
            {
                return Err(ApiError(StatusCode::CONFLICT, "inspection_changed"));
            }
        }

        let current = print::models(&app, Some(&workflow.printer_id)).await?;
        if current["capabilityRevision"] != payload["capabilityRevision"] {
            return Err(ApiError(
                StatusCode::CONFLICT,
                "capability_revision_changed",
            ));
        }
        if payload["expiresAt"].as_i64().unwrap_or(0) <= rip_storage::now() {
            return Err(ApiError(StatusCode::GONE, "preflight_expired"));
        }
        app.repo
            .insert_with_pipeline(
                &job,
                Some(id),
                Some(&ticket),
                Some((workflow.request_id, registration)),
            )
            .await
            .map_err(ApiError::from)
    }
    .await;
    if let Err(error) = result {
        let _ = tokio::fs::remove_dir_all(&directory).await;
        if let Some(job) = app.repo.committed_job(id).await? {
            return Ok((
                StatusCode::OK,
                Json(
                    json!({"jobId":job,"pipeline":app.repo.pipeline(workflow.request_id).await?,"replayed":true}),
                ),
            ));
        }
        return Err(error);
    }
    Ok((
        StatusCode::CREATED,
        Json(
            json!({"jobId":job_id,"pipeline":app.repo.pipeline(workflow.request_id).await?,"replayed":false}),
        ),
    ))
}
