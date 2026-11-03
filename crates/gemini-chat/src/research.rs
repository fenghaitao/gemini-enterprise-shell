//! Built-in Deep Research via the official streamAssist endpoint only.
use super::*;

const AGENT: &str = "deep_research";

impl GeminiEnterpriseClient {
    /// Request a plan once. Does not dispatch Start Research on the user's behalf.
    pub async fn plan_research(&self, request: ResearchPlanRequest) -> Result<ResearchResponse> {
        let mut assist = AssistRequest::new(request.topic);
        assist.data_stores = request.data_stores;
        assist.web_grounding = request.web_grounding;
        self.research(assist, ResearchPhase::Plan).await
    }

    /// Send Start Research once in the plan's session; can take 15+ minutes.
    /// No auth/HTTP/model fallback retries. After uncertainty, read the session
    /// or inspect the UI before considering another manually authorized request.
    pub async fn start_research(&self, request: ResearchStartRequest) -> Result<ResearchResponse> {
        let mut assist = AssistRequest::new("Start Research");
        assist.session = Some(request.session);
        assist.data_stores = request.data_stores;
        assist.web_grounding = request.web_grounding;
        self.research(assist, ResearchPhase::Start).await
    }

    /// Reconstruct the latest persisted assistant answer without generating.
    /// This does not attest that the session belongs to Deep Research; stage and
    /// agent fields may be absent from public session reads. Useful after a lost
    /// or rejected generation response; never automatically starts again.
    pub async fn get_research(&self, session: &str) -> Result<ResearchResponse> {
        let saved = self.get_session(session, true).await?;
        let answer = saved
            .turns
            .last()
            .and_then(|t| t.detailed_assist_answer.as_ref())
            .ok_or_else(|| {
                Error::Api("no detailed assistant answer in the latest saved turn".into())
            })?;
        let bytes =
            serde_json::to_vec(&json!([{"sessionInfo":{"session":saved.name},"answer":answer}]))
                .map_err(|_| Error::Api("invalid saved assistant answer".into()))?;
        self.parse_research(&bytes, Some(saved.name), ResearchPhase::Read)
            .map_err(|err| match err {
                Error::Assist { reason, .. } => Error::Api(format!(
                    "saved research answer is not ready or invalid: {reason}"
                )),
                Error::Skipped { .. } => Error::Api("saved research answer was skipped".into()),
                other => other,
            })
    }

    fn research_body(&self, request: &AssistRequest) -> Result<(Value, Option<String>)> {
        let (mut body, session) = self.request_body(request)?;
        body["agentsSpec"] = json!({"agentSpecs": [{"agentId": AGENT}]});
        Ok((body, session))
    }

    async fn research(
        &self,
        request: AssistRequest,
        phase: ResearchPhase,
    ) -> Result<ResearchResponse> {
        let (body, session) = self.research_body(&request)?;
        let bytes = self
            .send_assist(&body, session.clone(), self.config.research_timeout)
            .await?;
        self.parse_research(&bytes, session, phase)
    }

    fn parse_research(
        &self,
        bytes: &[u8],
        requested_session: Option<String>,
        phase: ResearchPhase,
    ) -> Result<ResearchResponse> {
        // Reuse the chat parser's bounded deserializer and strict session, answer,
        // terminal-state and frame-order validation. Grounding is validated below
        // per content block (terminal replies can repeat each block's metadata).
        let Frames(frames) = serde_json::from_slice(bytes).map_err(|_| {
            assist_error(
                "invalid or incomplete research JSON/schema",
                requested_session.clone(),
            )
        })?;
        let mut validation: Value = serde_json::from_slice(bytes)
            .map_err(|_| assist_error("invalid research JSON", requested_session.clone()))?;
        for frame in validation
            .as_array_mut()
            .expect("Frames validated an array")
        {
            if let Some(replies) = frame["answer"]["replies"].as_array_mut() {
                for reply in replies {
                    if let Some(grounded) = reply["groundedContent"].as_object_mut() {
                        grounded.remove("textGroundingMetadata");
                    }
                }
            }
        }
        let checked = self.parse_answer(
            &serde_json::to_vec(&validation)
                .map_err(|_| assist_error("invalid research JSON", requested_session.clone()))?,
            None,
            requested_session,
        )?;
        let session = Some(checked.session.clone());
        let mut contents: Vec<ResearchContent> = Vec::new();
        // Include hidden-thought slots only for aligning terminal snapshots; never
        // retain their text, files, stage metadata or grounding in results.
        let mut slots: Vec<Option<usize>> = Vec::new();
        for frame in frames {
            let Some(answer) = frame.answer else { continue };
            // v1's terminal snapshot mirrors all streamed replies, omitting their
            // text/file but repeating metadata in the same reply order.
            let snapshot = answer.state.as_deref() == Some("SUCCEEDED")
                && !contents.is_empty()
                && answer.replies.len() == slots.len()
                && answer
                    .replies
                    .iter()
                    .all(|r| r.grounded_content.content.is_none());
            for (index, reply) in answer.replies.into_iter().enumerate() {
                let grounded = reply.grounded_content;
                if snapshot {
                    if let Some(slot) = slots[index] {
                        let block = &mut contents[slot];
                        merge_label(
                            &mut block.kind,
                            grounded.content_metadata.and_then(|m| m.content_kind),
                            &session,
                        )?;
                        merge_label(&mut block.agent, reply.agent, &session)?;
                        if let Some(meta) = grounded.text_grounding_metadata {
                            add_grounding(block, meta, &session)?;
                        }
                    }
                    continue;
                }
                let Some(content) = grounded.content else {
                    if grounded.text_grounding_metadata.is_some() {
                        return Err(assist_error(
                            "unattached research grounding metadata",
                            session,
                        ));
                    }
                    continue;
                };
                if content.thought {
                    slots.push(None);
                    continue;
                }
                slots.push(Some(contents.len()));
                let mut block = ResearchContent {
                    kind: grounded.content_metadata.and_then(|m| m.content_kind),
                    agent: reply.agent,
                    text: content.text,
                    file: content.file,
                    grounding: Vec::new(),
                    unattributed_segments: Vec::new(),
                };
                if let Some(meta) = grounded.text_grounding_metadata {
                    add_grounding(&mut block, meta, &session)?;
                }
                contents.push(block);
            }
        }
        let report_indices: Vec<_> = contents
            .iter()
            .enumerate()
            .filter(|(_, c)| {
                c.kind.as_deref() == Some("RESEARCH_REPORT")
                    && c.text.as_ref().is_some_and(|t| !t.trim().is_empty())
            })
            .map(|(i, _)| i)
            .collect();
        // Stage labels may be present on intermediate Q&A but absent from the
        // actual final text. Prefer unlabelled text whenever no verified report
        // exists, regardless of whether other blocks have labels.
        let final_text_candidate_index = if report_indices.is_empty() {
            contents
                .iter()
                .rposition(|c| {
                    c.kind.is_none() && c.text.as_ref().is_some_and(|t| !t.trim().is_empty())
                })
                .or_else(|| {
                    contents
                        .iter()
                        .rposition(|c| c.text.as_ref().is_some_and(|t| !t.trim().is_empty()))
                })
        } else {
            None
        };
        let mut warnings: Vec<_> = checked
            .warnings
            .into_iter()
            .filter(|w| !w.starts_with("No native grounding"))
            .collect();
        if final_text_candidate_index.is_some() {
            warnings.push("No nonempty service-labelled RESEARCH_REPORT returned; research-stage labels may be incomplete or absent. The selected text block is only an unverified final-text candidate, not an attested plan/report, even if it has an intermediate stage label. HTTP success alone does not prove agent routing. All visible blocks are preserved.".into());
        }
        if phase != ResearchPhase::Plan && report_indices.is_empty() {
            warnings.push("No explicitly labelled RESEARCH_REPORT returned. Inspect the saved session/research UI before treating this result as a completed report; do not automatically start again.".into());
        }
        if !contents
            .iter()
            .any(|c| c.grounding.iter().any(|g| !g.references.is_empty()))
        {
            warnings.push("No native grounding references returned; prose citations are not structured citations.".into());
        }
        let unattributed: usize = contents.iter().map(|b| b.unattributed_segments.len()).sum();
        if unattributed > 0 {
            warnings.push(format!("{unattributed} native segment(s) contained -1 reference markers; preserved in unattributed_segments, NOT exposed as validated citations."));
        }
        Ok(ResearchResponse {
            session: checked.session,
            answer_resource: checked.answer_resource,
            requested_agent: (phase != ResearchPhase::Read).then(|| AGENT.into()),
            requested_phase: phase,
            contents,
            report_indices,
            final_text_candidate_index,
            invocation_tools: checked.invocation_tools,
            warnings,
        })
    }
}

fn merge_label(
    target: &mut Option<String>,
    value: Option<String>,
    session: &Option<String>,
) -> Result<()> {
    if let Some(value) = value {
        if target.as_ref().is_some_and(|existing| existing != &value) {
            return Err(assist_error(
                "conflicting research stage/agent metadata",
                session.clone(),
            ));
        }
        *target = Some(value);
    }
    Ok(())
}

fn add_grounding(
    block: &mut ResearchContent,
    mut meta: TextGroundingMetadata,
    session: &Option<String>,
) -> Result<()> {
    let text = block.text.as_deref().unwrap_or("");
    let mut valid = Vec::new();
    let mut unattributed = Vec::new();
    for segment in meta.segments {
        let mut check = segment.clone();
        let has_marker = segment
            .get("referenceIndices")
            .and_then(Value::as_array)
            .is_some_and(|indices| indices.iter().any(|i| i.as_i64() == Some(-1)));
        if has_marker {
            // Observed in a real completed report (2026-11-02). -1 cannot
            // identify a source. Quarantine the whole original segment, even
            // if it also names valid references; never invent an attribution.
            let indices = segment["referenceIndices"]
                .as_array()
                .expect("marker requires array");
            check["referenceIndices"] = Value::Array(
                indices
                    .iter()
                    .filter(|i| i.as_i64() != Some(-1))
                    .cloned()
                    .collect(),
            );
        }
        if !segment_is_valid(&check, meta.references.len(), text) {
            return Err(assist_error(
                "invalid research grounding indices or UTF-8 byte offsets",
                session.clone(),
            ));
        }
        if has_marker {
            unattributed.push(segment);
        } else {
            valid.push(segment);
        }
    }
    meta.segments = valid;
    let batch = block
        .grounding
        .iter()
        .position(|g| g == &meta)
        .unwrap_or_else(|| {
            block.grounding.push(meta);
            block.grounding.len() - 1
        });
    for segment in unattributed {
        let segment = UnattributedResearchSegment {
            grounding_batch: batch,
            segment,
        };
        if !block.unattributed_segments.contains(&segment) {
            block.unattributed_segments.push(segment);
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "research_tests.rs"]
mod tests;
