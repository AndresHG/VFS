//! OpenAI Responses API (VFS-28 R4, R6.1, R7).
//!
//! OpenAI has no citation block for passages the caller supplies, so passages are numbered in
//! the input and the answer is strict JSON `{answer, citations[{passage_id, quote}]}` (R7).
//! `store` is off: nothing is kept on OpenAI's side beyond its abuse logs.

use serde::Deserialize;
use serde_json::{json, Value};

use crate::cost::{models_for, Usage};
use crate::error::AiError;
use crate::http::{Http, SseReader, SHORT_CALL_TIMEOUT, STREAM_TIMEOUT};
use crate::provider::{
    rewrite_schema, Answer, AnswerBlock, AnswerEvent, AnswerRequest, CancelToken, ChatModel,
    Provider, RawCitation, Rewrite, RewriteRequest, ANSWER_MAX_TOKENS, REWRITE_MAX_TOKENS,
};

/// Reasoning effort for both calls: none, the least the models take (D7, "thinking off or
/// minimal"). `gpt-6-luna` rejects `minimal` with a 400 (first live run, VFS-28 AC-12).
const REASONING_EFFORT: &str = "none";

pub(crate) struct OpenAi {
    http: Http,
}

impl OpenAi {
    pub fn new(http: Http) -> Self {
        OpenAi { http }
    }
}

fn answer_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "answer": {"type": "string"},
            "citations": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "passage_id": {"type": "integer"},
                        "quote": {"type": "string"},
                    },
                    "required": ["passage_id", "quote"],
                    "additionalProperties": false,
                },
            },
        },
        "required": ["answer", "citations"],
        "additionalProperties": false,
    })
}

/// The passages as numbered data, one block each: `[P3] Contrato.pdf · p. 4` then the text.
fn numbered_passages(request: &AnswerRequest) -> String {
    let mut out = String::new();
    for (i, p) in request.passages.iter().enumerate() {
        out.push_str(&format!("[P{i}] {}\n{}\n\n", p.title, p.text));
    }
    out
}

/// The answer call's body (R7, R7a, R9): passages in the first user message, never in
/// `instructions`; earlier turns of the thread; strict JSON output.
pub(crate) fn answer_body(request: &AnswerRequest, model: &str) -> Value {
    let mut questions = request.earlier.iter().map(|t| t.question.as_str());
    let first_question = questions.next().unwrap_or(&request.question);
    let mut input = vec![json!({
        "role": "user",
        "content": [
            {"type": "input_text", "text": numbered_passages(request)},
            {"type": "input_text", "text": first_question},
        ],
    })];
    for (i, turn) in request.earlier.iter().enumerate() {
        input.push(json!({"role": "assistant", "content": turn.answer}));
        let next = request
            .earlier
            .get(i + 1)
            .map_or(request.question.as_str(), |t| &t.question);
        input.push(json!({"role": "user", "content": next}));
    }
    json!({
        "model": model,
        "instructions": request.system_prompt,
        "input": input,
        "text": {"format": {"type": "json_schema", "name": "answer", "schema": answer_schema(), "strict": true}},
        "max_output_tokens": ANSWER_MAX_TOKENS,
        "reasoning": {"effort": REASONING_EFFORT},
        "stream": true,
        "store": false,
    })
}

/// `input_tokens` counts cached tokens too; [`Usage::input_tokens`] does not.
fn usage_of(v: &Value) -> Usage {
    let input = v["input_tokens"].as_u64().unwrap_or(0);
    let cached = v["input_tokens_details"]["cached_tokens"]
        .as_u64()
        .unwrap_or(0);
    Usage {
        input_tokens: input.saturating_sub(cached),
        output_tokens: v["output_tokens"].as_u64().unwrap_or(0),
        cache_read_tokens: cached,
        cache_write_tokens: 0,
    }
}

/// The text of a finished response, or why there is none.
fn output_text(response: &Value) -> Result<String, AiError> {
    let mut text = String::new();
    for item in response["output"].as_array().into_iter().flatten() {
        if item["type"] != "message" {
            continue;
        }
        for part in item["content"].as_array().into_iter().flatten() {
            match part["type"].as_str() {
                Some("output_text") => text.push_str(part["text"].as_str().unwrap_or("")),
                Some("refusal") => return Err(AiError::BadResponse("the model refused".into())),
                _ => {}
            }
        }
    }
    match response["status"].as_str() {
        Some("completed") | None => Ok(text),
        Some(status) => Err(AiError::BadResponse(format!(
            "response {status}: {}",
            response["incomplete_details"]["reason"]
        ))),
    }
}

#[derive(Deserialize)]
struct JsonAnswer {
    answer: String,
    citations: Vec<JsonCitation>,
}

#[derive(Deserialize)]
struct JsonCitation {
    passage_id: i64,
    quote: String,
}

impl ChatModel for OpenAi {
    fn provider(&self) -> Provider {
        Provider::OpenAi
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
        let model = models_for(Provider::OpenAi).rewrite;
        let body = json!({
            "model": model,
            "instructions": request.system_prompt,
            "input": request.question,
            "text": {"format": {"type": "json_schema", "name": "rewrite", "schema": rewrite_schema(), "strict": true}},
            "max_output_tokens": REWRITE_MAX_TOKENS,
            "reasoning": {"effort": REASONING_EFFORT},
            "store": false,
        });
        let response = self.http.send(cancel, SHORT_CALL_TIMEOUT, || {
            self.http.post("/v1/responses").json(&body)
        })?;
        let v: Value = response
            .json()
            .map_err(|e| AiError::BadResponse(e.to_string()))?;
        let usage = usage_of(&v["usage"]);
        let text = output_text(&v)?;
        let rewrite = serde_json::from_str(&text)
            .map_err(|_| AiError::BadResponse("rewrite is not the schema".into()))?;
        Ok((rewrite, text, usage))
    }

    fn answer(
        &self,
        request: &AnswerRequest,
        cancel: &CancelToken,
        on_event: &mut dyn FnMut(AnswerEvent),
    ) -> Result<Answer, AiError> {
        let models = models_for(Provider::OpenAi);
        let model = if request.cheap {
            models.cheap_answer
        } else {
            models.answer
        };
        let body = answer_body(request, model);
        let response = self.http.send(cancel, STREAM_TIMEOUT, || {
            self.http.post("/v1/responses").json(&body)
        })?;
        let mut reader = SseReader::new(response);
        let mut finished: Option<Value> = None;
        loop {
            if cancel.is_cancelled() {
                return Err(AiError::Cancelled { usage: None });
            }
            let Some(event) = reader.next_event()? else {
                break;
            };
            let v: Value = serde_json::from_str(&event.data).unwrap_or(Value::Null);
            match v["type"].as_str().unwrap_or(&event.event) {
                "response.completed" | "response.incomplete" => {
                    finished = Some(v["response"].clone());
                    break;
                }
                "response.failed" => {
                    let body = json!({"error": v["response"]["error"]}).to_string();
                    return Err(self.http.stream_error(&body));
                }
                "error" => {
                    let body = json!({"error": v.get("error").unwrap_or(&v)}).to_string();
                    return Err(self.http.stream_error(&body));
                }
                _ => {}
            }
        }
        let response = finished
            .ok_or_else(|| AiError::BadResponse("stream ended before completion".into()))?;
        let usage = usage_of(&response["usage"]);
        let text = output_text(&response)?;
        let parsed: JsonAnswer = serde_json::from_str(&text)
            .map_err(|_| AiError::BadResponse("answer is not the schema".into()))?;
        on_event(AnswerEvent::Text(parsed.answer.clone()));
        let citations = parsed
            .citations
            .into_iter()
            .map(|c| RawCitation {
                passage: usize::try_from(c.passage_id).unwrap_or(usize::MAX),
                quote: c.quote,
            })
            .collect();
        Ok(Answer {
            model: model.to_owned(),
            blocks: vec![AnswerBlock {
                text: parsed.answer,
                citations,
            }],
            usage,
            stop_reason: response["status"].as_str().map(str::to_owned),
        })
    }
}
