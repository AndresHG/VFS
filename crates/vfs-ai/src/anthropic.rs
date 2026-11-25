//! Anthropic Messages API (VFS-28 R4, R6.1, R7).
//!
//! Shapes follow Anthropic's API reference for version `2026-10-21`: structured output through
//! `output_config.format` for the rewrite only, and `search_result` blocks with citations for
//! the answer (D8). Thinking is off: `disabled` on Haiku 5.5, `between_tools` on Sonnet 5.5,
//! which rejects `disabled`.

use serde_json::{json, Value};

use crate::cost::{models_for, Usage, CLAUDE_OPUS, CLAUDE_SONNET};
use crate::error::AiError;
use crate::http::{Http, SseReader, SHORT_CALL_TIMEOUT, STREAM_TIMEOUT};
use crate::provider::{
    rewrite_schema, Answer, AnswerBlock, AnswerEvent, AnswerRequest, CancelToken, ChatModel,
    Provider, RawCitation, Rewrite, RewriteRequest, ANSWER_MAX_TOKENS, REWRITE_MAX_TOKENS,
};

pub(crate) struct Anthropic {
    http: Http,
}

impl Anthropic {
    pub fn new(http: Http) -> Self {
        Anthropic { http }
    }
}

/// The `thinking` setting that turns thinking off on `model`.
fn thinking_off(model: &str) -> Value {
    if model == CLAUDE_SONNET {
        json!({"type": "between_tools"})
    } else {
        json!({"type": "disabled"})
    }
}

/// Sets the least thinking `model` takes on an answer body. Opus 5.5 cannot turn thinking off
/// (a 400), so it thinks adaptively at `low` effort, with room for that thinking in
/// `max_tokens`; every other model turns it off (D7, VFS-46).
fn set_least_thinking(body: &mut Value, model: &str) {
    if model == CLAUDE_OPUS {
        body["output_config"] = json!({"effort": "low"});
        body["max_tokens"] = json!(ANSWER_MAX_TOKENS * 4);
    } else {
        body["thinking"] = thinking_off(model);
    }
}

/// The answer call's body (R7, R7a, R9): passages as `search_result` blocks with citations on,
/// ahead of the first question; earlier turns of the thread; no structured output.
pub(crate) fn answer_body(request: &AnswerRequest, model: &str) -> Value {
    let mut first: Vec<Value> = request
        .passages
        .iter()
        .map(|p| {
            json!({
                "type": "search_result",
                "source": p.source,
                "title": p.title,
                "content": [{"type": "text", "text": p.text}],
                "citations": {"enabled": true},
            })
        })
        .collect();
    let mut questions = request.earlier.iter().map(|t| t.question.as_str());
    let first_question = questions.next().unwrap_or(&request.question);
    first.push(json!({"type": "text", "text": first_question}));
    let mut messages = vec![json!({"role": "user", "content": first})];
    for (i, turn) in request.earlier.iter().enumerate() {
        messages.push(json!({"role": "assistant", "content": turn.answer}));
        let next = request
            .earlier
            .get(i + 1)
            .map_or(request.question.as_str(), |t| &t.question);
        messages.push(json!({"role": "user", "content": next}));
    }
    let mut body = json!({
        "model": model,
        "max_tokens": ANSWER_MAX_TOKENS,
        "stream": true,
        "system": request.system_prompt,
        "messages": messages,
    });
    set_least_thinking(&mut body, model);
    body
}

fn usage_of(v: &Value) -> Usage {
    let n = |k: &str| v[k].as_u64().unwrap_or(0);
    Usage {
        input_tokens: n("input_tokens"),
        output_tokens: n("output_tokens"),
        cache_read_tokens: n("cache_read_input_tokens"),
        cache_write_tokens: n("cache_creation_input_tokens"),
    }
}

impl ChatModel for Anthropic {
    fn provider(&self) -> Provider {
        Provider::Anthropic
    }

    fn validate_key(&self) -> Result<(), AiError> {
        self.http
            .send(&CancelToken::new(), SHORT_CALL_TIMEOUT, || {
                self.http.get("/v1/models")
            })
            .map(drop)
    }

    fn rewrite(
        &self,
        request: &RewriteRequest,
        cancel: &CancelToken,
    ) -> Result<(Rewrite, String, Usage), AiError> {
        let model = models_for(Provider::Anthropic).rewrite;
        let body = json!({
            "model": model,
            "max_tokens": REWRITE_MAX_TOKENS,
            "system": request.system_prompt,
            "thinking": thinking_off(model),
            "output_config": {"format": {"type": "json_schema", "schema": rewrite_schema()}},
            "messages": [{"role": "user", "content": request.question}],
        });
        let response = self.http.send(cancel, SHORT_CALL_TIMEOUT, || {
            self.http.post("/v1/messages").json(&body)
        })?;
        let v: Value = response
            .json()
            .map_err(|e| AiError::BadResponse(e.to_string()))?;
        let usage = usage_of(&v["usage"]);
        let text: String = v["content"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|b| b["type"] == "text")
            .filter_map(|b| b["text"].as_str())
            .collect();
        let rewrite = serde_json::from_str(&text).map_err(|_| {
            AiError::BadResponse(format!(
                "rewrite is not the schema (stop {})",
                v["stop_reason"]
            ))
        })?;
        Ok((rewrite, text, usage))
    }

    fn answer(
        &self,
        request: &AnswerRequest,
        cancel: &CancelToken,
        on_event: &mut dyn FnMut(AnswerEvent),
    ) -> Result<Answer, AiError> {
        let model = request.answer_model(Provider::Anthropic);
        let body = answer_body(request, model);
        let response = self.http.send(cancel, STREAM_TIMEOUT, || {
            self.http.post("/v1/messages").json(&body)
        })?;
        let mut reader = SseReader::new(response);
        let mut usage = Usage::default();
        let mut reported = false;
        let mut blocks: Vec<AnswerBlock> = Vec::new();
        let mut current: Option<AnswerBlock> = None;
        let mut stop_reason = None;
        loop {
            if cancel.is_cancelled() {
                return Err(AiError::Cancelled {
                    usage: reported.then_some(usage),
                });
            }
            let Some(event) = reader.next_event()? else {
                break;
            };
            let v: Value = serde_json::from_str(&event.data).unwrap_or(Value::Null);
            match v["type"].as_str().unwrap_or(&event.event) {
                "message_start" => {
                    usage = usage_of(&v["message"]["usage"]);
                    reported = true;
                }
                "content_block_start" => {
                    let block = &v["content_block"];
                    current = (block["type"] == "text").then(|| AnswerBlock {
                        text: block["text"].as_str().unwrap_or("").to_owned(),
                        citations: Vec::new(),
                    });
                }
                "content_block_delta" => {
                    let delta = &v["delta"];
                    let Some(block) = current.as_mut() else {
                        continue;
                    };
                    match delta["type"].as_str() {
                        Some("text_delta") => {
                            let text = delta["text"].as_str().unwrap_or("");
                            block.text.push_str(text);
                            on_event(AnswerEvent::Text(text.to_owned()));
                        }
                        Some("citations_delta") => {
                            let c = &delta["citation"];
                            if let Some(index) = c["search_result_index"].as_u64() {
                                block.citations.push(RawCitation {
                                    passage: usize::try_from(index).unwrap_or(usize::MAX),
                                    quote: c["cited_text"].as_str().unwrap_or("").to_owned(),
                                });
                            }
                        }
                        _ => {}
                    }
                }
                "content_block_stop" => blocks.extend(current.take()),
                "message_delta" => {
                    let u = &v["usage"];
                    if let Some(out) = u["output_tokens"].as_u64() {
                        usage.output_tokens = out;
                    }
                    if let Some(input) = u["input_tokens"].as_u64() {
                        usage.input_tokens = input;
                    }
                    if let Some(s) = v["delta"]["stop_reason"].as_str() {
                        stop_reason = Some(s.to_owned());
                    }
                }
                "message_stop" => break,
                "error" => return Err(self.http.stream_error(&event.data)),
                _ => {}
            }
        }
        blocks.extend(current.take());
        if stop_reason.is_none() {
            return Err(AiError::BadResponse(
                "stream ended before message_stop".into(),
            ));
        }
        Ok(Answer {
            model: model.to_owned(),
            blocks,
            usage,
            stop_reason,
        })
    }
}
