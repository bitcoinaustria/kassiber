# AI Reference

The desktop Assistant and `kassiber chat` use the same provider configuration,
tool loop, and daemon authority. External coding/terminal assistants can use
the [checked-in CLI skill](../../skills/kassiber/SKILL.md).

Accounting, imports, matching, and reports remain deterministic and work without
AI. Models can propose classifications, extract document fields, and help resolve
missing evidence; those outputs require the same validation and review as other
inputs.

## HTTP transport

HTTP providers speak one of two OpenAI wire formats, the Responses API
(`POST /v1/responses`) or Chat Completions (`POST /v1/chat/completions`), with
streaming and typed function calls, and list models with `GET /v1/models`.
Kassiber's daemon runs the tool loop either way: it offers the allowlisted tool
menu, validates each requested call, asks for consent where needed, runs it and
returns the redacted result. Inside Kassiber the conversation is a list of
typed Responses Items. The client translates it to the provider's format at
the transport boundary and turns both streams into the same events, so the
tool loop never sees which format a provider speaks.

### Choosing the format

Each HTTP provider has an `api_format` setting, `auto` by default, and a
separately stored `detected_api_format`:

- `responses` or `chat_completions` pins the format, and Kassiber never
  switches away from it.
- `auto` speaks the detected format, else the Responses API (Ollama, oMLX and
  OpenAI serve it). When an `auto` provider with nothing detected gets HTTP
  404, 405 or 501 (Not Implemented) from `/responses`, or a 400 whose body says
  the endpoint itself is unknown (such as LM Studio's "Unexpected endpoint or
  method"), the same turn
  goes once to `/chat/completions` at the same base URL, the endpoint the user
  already chose, and an answer there is stored as `detected_api_format`. A 400
  matches only on explicit endpoint or route wording, never on a bare "not
  found", which usually describes the request's content. A response that names
  a model or tools, any other error, and a pinned format never fall back.
- Changing the base URL clears the detection. CLI agents (`*-cli://`) have
  their own protocol and take only `auto`: a provider moved to a CLI locator
  without a format of its own goes back to `auto` (the desktop form sends
  `api_format: "auto"` and clears the detection), and an explicit HTTP format
  for a CLI locator is refused.

Provider envelopes (`ai.providers.*`, `kassiber ai providers ...`) carry
`api_format`, `detected_api_format`, and `effective_api_format`, the format the
next chat speaks (`null` for a CLI agent). The CLI sets the format with
`kassiber ai providers create|update NAME --api-format
auto|responses|chat-completions`; `--clear detected_api_format` forgets a
detection.

### Connection test

Settings **Test connection** (`ai.test_connection`) and `kassiber ai providers
test NAME` list the endpoint's models and then send one fixed request in the
provider's format: the prompt `Reply with OK`, at most 16 output tokens
(OpenAI's Responses API refuses fewer), no book data, `store: false` on
Responses, and one trivial tool, `connection_check`, that the model is not
asked to call. `auto` tries the Responses API and then Chat Completions when
the endpoint does not speak it; an endpoint that refuses the tool is asked once
more without it, whether it refuses the request or reports the refusal as an
error inside its stream, as OpenRouter can. A Chat Completions endpoint that
refuses `store: false` is asked once more without it, to tell an endpoint that
works but refuses the field from a broken one; that is safe only because the
test sends nothing but its fixed prompt, and the result says so with
`store_accepted: false`. The test changes no setting: chats keep sending
`store: false` and fail with `ai_store_refused` until the user chooses
`omit_store_field`. Each retry happens at most once, so a format takes at most
three test requests. An error inside an accepted stream keeps its numeric code
as `details.status` (with `details.in_stream: true`), but it never counts as a
missing format or route. The request needs a model: the one given (the desktop form
sends its default model field), else the saved default model, else the only
model the endpoint lists. With none, nothing guesses (an embedding model would
fail the test), and a model listing alone is not reported as success: the
result says `probe.reason: "no_model"`, the desktop form says that no reply
was checked and asks for a model (never success), and `kassiber ai providers
test` fails with `ai_model_required`. Listing waits at most 10 seconds; the
test request waits up to 30 so a local model can load.

The `ai.test_connection` result keeps `base_url`, `model_count`, `models`, and
`check_kind`, and adds `api_format` (the setting tested, `null` for a CLI
agent), `detected_api_format` (the format that answered, or `null`),
`tools_accepted` (`true`, `false`, or `null` when nothing was sent),
`store_accepted` (`true`, `false` when a Chat Completions endpoint refused
`store: false` and answered the fixed test prompt without it, or `null` when
not asked: no reply test, or a provider that already omits the field),
`detection_saved`, and `probe: {status: "ok"|"skipped", reason:
null|"no_model"|"cli_provider", model, finish_reason}`. Testing a saved
provider's own base URL stores the detection. Testing unsaved form values
returns it, and the form saves it with the row through `detected_api_format`
on `ai.providers.create` or `ai.providers.update`. The arguments are
`base_url`, optional `api_key`, `kind`, `provider`, `api_format`, `model`,
`omit_store_field` (the form's choice, else the saved provider's, but only
for the saved address: a test of another address, like saving it, starts
without the exemption, and a CLI target ignores the saved provider's
HTTP-only settings), and `acknowledged`. A failed test request fails the
test with its error, like a failed model listing.

Because the test sends a prompt, an off-device target needs the same
acknowledgement as chat before anything is requested, model listing
included. Only a `local` target whose URL is loopback HTTP is on this device;
a `local` claim for another URL, a remote or TEE kind, a CLI agent, and
unsaved values without a kind at an address off this device are off-device.
Such a test runs with `acknowledged: true` (the provider form's checkbox), or
when the named saved provider is itself acknowledged as remote or TEE. A local
row's implicit acknowledgement does not count, as when a local provider
becomes remote. Otherwise the daemon refuses with `ai_remote_ack_required`
and sends nothing. A switched-off saved provider is refused as before, and
`kassiber ai providers test` requires the saved provider's acknowledgement.

Unlike **Check**, the test is not refused with `ai_provider_changed` when a
named provider's row has moved: its `base_url` is the endpoint to test, not an
expectation about the row, and it contacts only that URL. What it takes from a
named row cannot follow the row elsewhere. A stored API key belongs to its
origin (scheme, host and port, lowercased, default ports dropped): it is used
only when the tested URL is on the row's origin, a path change included, and a
row with a key on another origin refuses the test before anything is
contacted. The provider store keeps the same rule: an update that moves
`base_url` to another origin without a new key drops the stored key, and one
that changes only the path keeps it. The saved model and format are
only fallbacks for arguments the caller left out, and a detection is written
only onto a row that still points at the tested URL, checked in the same
database update.

### Reasoning effort over HTTP

HTTP endpoints broadly take `low`, `medium`, or `high`. Kassiber's `xhigh`,
`max`, and `ultra`, which exist for the CLI agents, go out as `high`, and
`auto` sends nothing. The Responses API gets `reasoning: {effort}`. Kassiber
does not ask for reasoning summaries: no model listing says which models
produce one, and OpenAI refuses them for an unverified organization, which
would fail the turn. Endpoints that stream reasoning on their own, such as
Ollama and oMLX, still show it. Chat Completions gets the standard
`reasoning_effort`, except on `openrouter.ai`, which documents a
`reasoning: {effort}` object for it.

When an endpoint refuses the effort (a 400 or 422 that names reasoning,
effort, summary, or thinking), Kassiber sends the request once more without it
and leaves it out for the rest of the turn. Reasoning replayed from earlier
rounds is a separate part and stays: DeepSeek refuses an effort but needs its
`reasoning_content` back in the next tool round. Only a refusal that names
the replayed reasoning (`reasoning_content`, `reasoning_details`, a reasoning
Item), or a reasoning refusal of a request that carried no effort, leaves the
replay out, once; a refusal naming neither drops the effort first. The answer then carries the
`reasoning_effort_unavailable` notice through the same allowlist as
`fast_mode_unavailable`: an `ai.chat.status` record, the terminal provenance
`notices` (and so `kassiber chat --machine`), stored history, and the
desktop's notice under the answer.

Model listings feed the effort menu. A model whose `supported_parameters`
include `reasoning`, `include_reasoning`, or `reasoning_effort` (OpenRouter
lists them) reports `supports_reasoning_effort: true` and `reasoning_efforts`
with the HTTP levels it takes, narrowed by OpenRouter's
`reasoning.supported_efforts`. `supports_tools` is `true` or `false` from the
same list, or `null` when a listing says nothing, as OpenAI's does.
`input_modalities` and `output_modalities` come from OpenRouter's
`architecture`, as bounded lowercase lists.

The desktop's effort menu offers an HTTP model only the levels in its
`reasoning_efforts`, all three when it takes an effort without naming any, and
none when its listing does not say it takes one (as OpenAI's never does) or
before **Check models** has listed it. Levels a listing names elsewhere, such
as `capabilities.reasoning_efforts: ["xhigh"]`, are not offered, and an HTTP
provider's default effort on its Settings card is bounded to low, medium and
high the same way. A stored default the model does not offer moves to the
nearest offered level, as for CLI agents.

### Stateless requests

Kassiber deliberately does **not** enable provider-managed conversation state.
Responses requests set `store: false`; system guidance is sent through
`instructions`, and the bounded tool loop replays the round's typed output
Items in memory. With nothing stored, a reasoning Item can only be replayed
through its encrypted content: a request that offers tools, or sets an
effort, asks for `include: ["reasoning.encrypted_content"]`. A reasoning model
reasons whatever the effort, and with OpenAI the effort is usually auto, since
its listing names none, so the effort alone would leave a tool round's
follow-up without the reasoning it needs. A reasoning Item without encrypted
content is left out instead of being sent as an id the provider never stored.
An endpoint that refuses `include` gets the request once more without it, and
later rounds leave it out.

Chat Completions requests stay minimal for broad compatibility: `model`,
`messages`, `stream`, `store: false`, `tools` and `tool_choice: "auto"` when
there are tools, the generation options the caller set (such as `temperature`
or `max_tokens`), and the effort. OpenAI stores Chat Completions by default for
new accounts, so `store: false` is sent like on the Responses API.

Kassiber never resends a request without `store: false` because of an error
message: what an error means cannot be trusted to keep the user's prompt and
tool results out of a provider's default retention. A 400 or 422 to a request
that carried `store: false`, whose structured field (`error.param`, or a
validation error's `loc`) names `store`, or whose message names it when there
is no structured field, fails the turn with `ai_store_refused` ("This endpoint
refused Kassiber's no-retention request (store: false)") and a hint to run the
connection test. That holds for both formats and whatever the error says,
including an error that also names other fields; request input an error
echoes back is never read. Other refusals are handled as before: the
`include`, effort, reasoning-replay and `max_tokens` retries each happen at
most once. Each reads what the error blames the same way as the store
refusal: from its structured field path when it has one, else from its
message, never from request data it echoes back, so a validation error about
`temperature` that echoes `include` or `reasoning` drops nothing. The
connection test's retry without its tool follows the same rule.

Leaving `store` out is only ever an explicit, visible choice. A provider's
`omit_store_field` setting (off by default; `--omit-store-field` on `kassiber
ai providers create|update`, `--send-store-field` to undo it, and the
**Advanced** checkbox in the desktop provider form) sends its Chat Completions
requests without the field, and the provider's own retention then applies to
prompts and tool results. The Responses API always sends `store: false`. CLI
agents take no such setting. The choice belongs to the endpoint it was made
for: an update that changes `base_url` without restating it goes back to the
default, as a detection is cleared, and so does the desktop form, which also
puts the format back to auto and drops the last test result when the address
changes ("No-retention exemption reset for the new address."); a test or save
straight after such a change stops so the user can review it.
Instructions become a system message, a round's function calls one assistant
message with `tool_calls`, and each result a `tool` message. Streamed tool
calls are put together strictly by `index`; two indices that share a call id
make the stream malformed, and the turn fails closed before anything runs.
Streamed calls run only after an explicit finish reason other than `length`
or `content_filter`. `stop` counts, since some servers (Ollama, llama.cpp) end
a complete tool-call answer that way and it means the model ended the answer
itself. A stream that sends call fragments and then `[DONE]` without any
finish reason runs none of them and fails the turn (`ai_unavailable`, reason
`no_finish_reason`); a text answer without one still stands, as `stop`. A bare
end of the connection never marks an answer complete: a Chat Completions
stream that ends with neither `[DONE]` nor a finish reason, or a Responses
stream that ends without `response.completed` or `response.incomplete`, fails
the turn (`ai_unavailable`, reason `stream_incomplete`, retryable), so no
partial text is shown or stored as a finished answer. Nor does a terminator
alone make an answer: a Chat Completions stream with no choice that carried
content, a tool call or a finish reason before `[DONE]`, or a
`response.completed` with no output and nothing streamed, fails with
`ai_request_invalid`, reason `no_answer`. The connection test reads that as no
answer in this format, so it detects nothing from an endpoint that never
replied. A stream that drops before finishing counts as an unserved format
only if none of its events had that format's shape; once the endpoint has
spoken Responses (or Chat Completions), a drop fails the test as retryable
`stream_incomplete` instead of resending the prompt in the other format and
recording the wrong one.
Reasoning that arrives in the non-standard `reasoning_content` (DeepSeek,
vLLM, llama.cpp) or `reasoning` (OpenRouter, Ollama) fields is shown as
reasoning, never as the answer, and is replayed to the next tool round under
the same field, since DeepSeek refuses a tool round without it. OpenRouter's
`reasoning_details` are merged and replayed whole. A model that refuses
`max_tokens` and names `max_completion_tokens`, as OpenAI's reasoning models
do, gets the request again with the latter.

Neither format sends OpenRouter's app attribution headers (`HTTP-Referer`,
`X-OpenRouter-Title`), which exist to list an app's usage in its public
rankings.

An answer cut off by the token limit or a content filter ends with
`finish_reason` `length` or `content_filter` in both formats (on the Responses
API that is the `response.incomplete` event), and the daemon runs no tool call
from it, whatever the provider: the turn ends there. Under
`response.incomplete` only a function call marked `completed` counts as
finished; one without a `status`, as some endpoints send, may be the one cut
off and is neither kept nor replayed.

Persisted chat history remains a Kassiber/SQLCipher concern and is sent again
as input only when the user resumes a session. This keeps the local-first
storage boundary and also works with providers that implement only stateless
APIs, such as OpenRouter's Responses API.

The implementation keeps those invariants in one client. Prepared tool-loop
context is an explicit typed input and cannot be combined with the legacy
message input accidentally. HTTP transport remains in `kassiber.ai.client`.
Fixed external CLI locators route through the bundled Node/TypeScript broker in
`ui-tauri/provider-broker`; Python supervises that JSONL process through
`kassiber.ai.broker_client`. Shared delta/request contracts remain isolated in
`kassiber.ai.contracts`.

### Building the broker from a source checkout

The broker ships as a single bundled `kassiber/ai/provider_broker/index.mjs`,
built by esbuild from `ui-tauri/provider-broker/src`. It is **not committed** —
release builds produce it before packaging. In a source checkout the CLI-backed
providers report `ai_unavailable` until you build it once:

```bash
pnpm --dir ui-tauri run broker:build
```

`pnpm --dir ui-tauri run build` runs that step first, so a desktop build already
covers it. Point `KASSIBER_AI_PROVIDER_BROKER` at another path to override which
script is spawned.

## Privacy warning

Treat AI prompts as sensitive accounting data.

Depending on what you send, prompts may contain:

- wallet labels
- addresses
- transaction notes
- imported document contents
- backend hostnames
- reconciliation context
- accounting or tax annotations

Normal Kassiber `backends ...` and `wallets ...` output is intentionally
narrowed for secret-bearing config values, but that is not a general privacy
guarantee. Do not paste raw credentials, raw private descriptors, Silent
Payments `sp()` / `spscan` scan material, wallet exports, or other sensitive
material into a remote model unless that is acceptable for your threat model.

If in doubt, keep inference local, or restrict the provider's agent access.

### Agent access

Each provider has an `agent_access` setting, `full` by default. With full
access the assistant gets every tool and unprojected result: custody-gap,
coverage and lineage reads and `ui.custody.review.apply`; review batches
with pairs, dismissals, component revisions, custody-gap actions and
acquisition kinds; chain-analysis
sources, acquisition and dataset tools; chain-analysis results with their
txids, outpoints and labels instead of opaque references; and an attached
file's cell values and name. `restricted` keeps an off-device provider to the
projected surface: those tools are denied with `agent_access_restricted`,
chain-analysis results are projected, and an attachment is reduced to headers,
counts and the column plan (its import plan likewise drops dates, amounts and
the filename). A provider on this device (kind `local` at a
loopback URL) always has full access.

Set it in the provider form (**Restrict what the assistant can use**) or with
`kassiber ai providers create|update --agent-access full|restricted`. The
daemon decides it once per chat in `_provider_has_full_agent_access`; a direct
or MCP caller without that decision is restricted. Neither mode sends
descriptors, xpubs, wallet configuration, secrets, local paths or URLs, or
offers raw shell, filesystem or CLI dispatch, and consent still gates every
write.

Codex, Claude, OpenCode, and GitHub Copilot CLI providers are
supported for convenience, but they are not a local-privacy guarantee. The
broker denies provider-native tools in layers — Claude loads no
user/project/local settings, empties its built-in tool set, disables slash
commands, and accepts only its temporary Kassiber MCP server; OpenCode serves
with `--pure` and a deny-all session permission plus exact Kassiber MCP allows;
Codex runs a read-only sandbox with network access off; the
[Agent Client Protocol agent](#agent-client-protocol-agents) starts with its
built-in tools removed — and any tool outside the advertised Kassiber catalog
aborts the turn.

Kassiber's own capability-scoped schemas do cross into these providers, through
their native typed-tool protocols: Codex `dynamicTools`, and an ephemeral MCP
server for Claude, OpenCode, and Copilot. Only the schemas and already-redacted results
traverse that bridge, over a unix socket inside a private 0700 directory; the
Python daemon stays the authority for capability selection, argument
validation, consent, execution, and the privacy receipt. Provider-native coding
tools remain disabled throughout. Because these providers route to their own
models, that is the point at which accounting data can leave the device —
choose them deliberately.

Codex exposes no tool-free profile, so a local read there can begin before the
abort lands; with its network disabled that content can only surface through
assistant text on a turn Kassiber is already failing. Kassiber reuses their normal local
authentication/config, telemetry, and model-provider routing without reading,
copying, persisting, or displaying provider credentials. Treat them as
off-device unless the underlying configuration proves local or confidential
inference.

## Provider setup

Local inference is the recommended default.

Kassiber seeds first-class local provider rows for:

- [Ollama](https://ollama.com/) at `http://localhost:11434/v1`
- [oMLX](https://omlx.ai/) at `http://127.0.0.1:8000/v1`

Ollama remains the default provider for compatibility, but oMLX appears as a
built-in local provider and Settings preset. Run the server (`ollama serve`, or
`omlx start` / the oMLX menu-bar app) and use **Test connection** in Settings
before saving a provider change.

For an HTTP provider, Settings **Test connection** lists its models and sends
one fixed test request to learn which API format answers and whether tools are
accepted (see [Connection test](#connection-test)); for a CLI agent it checks
the binary and its models. It does not run Kassiber's own tools, so an
explicitly started chat remains the final check. Endpoints that speak only Chat
Completions work through the detected or chosen format.

### Adding a provider in the desktop

**Add provider** in Settings → AI providers opens a wizard, laid out after
T3 Code's add-provider wizard, with a step track: **Provider**, **Connection**,
**Privacy** (only for a provider whose prompts leave this device) and
**Model**. Nothing contacts the provider until **Test connection** on the last
step, and no step needs a test to finish.

**Provider** first asks how the model is reached, which is also where prompts
go, each card with its posture badge:

- **On this computer**: Ollama, LM Studio, vLLM, llama.cpp, oMLX, or
  **Other compatible server**. Each local server shows the command that
  starts it the way the assistant needs, to copy; Kassiber never runs it.
  vLLM and llama.cpp refuse tools unless started with a tool parser
  (`--enable-auto-tool-choice --tool-call-parser …`) or `--jinja`.
- **Through your subscription app**: the Claude, Codex, OpenCode and GitHub
  Copilot CLI agents.
- **With an API key**: OpenAI (`https://api.openai.com/v1`), OpenRouter
  (`https://openrouter.ai/api/v1`), or **Other OpenAI-compatible API**.

"OpenAI-compatible" covers any server that speaks the OpenAI API; the two
"other" entries differ only in posture: the server on this computer starts
local and follows its address, the hosted API starts remote. A preset fills
the name, base URL, posture and notes, and says in one line what it is for;
none brings a model list. **Connection** holds the name, the address and the
API key (kept in the system keychain), with the posture, API format, store
choice and notes under **Advanced**. A new OpenAI or OpenRouter provider does
not get past it without an API key, and leaving it runs the same address
checks as the address field (below). **Privacy** says that prompts leave this
device and what does not (descriptors, xpubs, secrets and wallet files), and
holds the acknowledgement; the wizard does not move on without it. It also
holds **Restrict what the assistant can use**, unticked, which sets the
provider's [agent access](#agent-access) to restricted. **Model**
holds the default model and **Test connection**. Editing a provider opens the
same wizard at **Connection**, without the provider step, and saves from any
step.

The posture follows the URL one way only. When the URL field is left, the
Connection step is left, or the form is tested or saved, a local provider
whose address is not on this device, or a CLI agent, moves to remote with a
note saying why ("This address isn't on this device, so it's treated as
remote."), and the acknowledgement is asked for again; a save that moves it
returns to the Privacy step. On this device means `localhost`, `127.0.0.0/8`
or an IPv6 loopback, read as written like the daemon's own check, so `127.1`
is not loopback. The form never moves a provider to local and never changes a
remote or TEE choice.

To use an OpenRouter key, choose **With an API key** → **OpenRouter**, paste
the key, acknowledge, and press **Test connection** on the Model step.
OpenRouter lists hundreds of models, so without a default model the test lists
them but checks no reply ("Choose a model to check that it can answer."). The
Default model field then suggests the listed models that can run the
assistant, and testing again with one of them checks a reply, the format and
tools. OpenAI works the same way.

**Advanced** holds the API format: **Auto (recommended)**, **Responses API** or
**Chat Completions** (see [Choosing the format](#choosing-the-format)). It is
sent as `api_format` with the test and the saved row, opens on its own when a
provider has a format other than auto, and shows an edited provider's last
detection while the URL is the saved one; CLI agents have no format, so for
them Advanced holds only the posture and notes. It also holds the one explicit way to leave out
`store: false`: an unchecked-by-default box, "Send requests without
Kassiber's no-retention flag", whose hint says the provider's own retention
then applies to prompts and tool results. It is saved as `omit_store_field`,
opens Advanced on its own when saved, and is never ticked by the form. When a
test reports `store_accepted: false`, the form says the endpoint refuses the
flag and that chats with it will fail, and opens Advanced to show the box
without ticking it. After a test the form shows the model that answered, the format
that answered (detected under auto, as set otherwise), and whether tools were
accepted. An endpoint that answered only without tools gets a warning that the
assistant cannot read the book through it. A failed test shows the daemon's
error. Each test belongs to the values it was started with: editing the URL,
key, model or format hides its result, an answer that arrives after such an
edit, or after a newer test started (even for the same values, as on a
double click), is dropped, and only a detection for the
values being saved is saved with the row (`detected_api_format`). A newly
typed API key goes with the test for that one request and is stored only when
the provider is saved. A typed key belongs to the origin (scheme, host and
port) it was typed for: choosing another preset clears it, together with the
format choice, the store choice and the last test result, and moving the
address to another origin clears it with a note ("API key cleared because the
address changed."), checked when the field is left and before every test or
save, so it is never sent to the new origin. A path change on the same origin
keeps it. A saved key follows the same origin rule in the daemon. An off-device test waits for the acknowledgement
checkbox ("Testing sends a short prompt to this provider. Acknowledge that
prompts may leave this device first.") unless the saved provider is already
acknowledged as off-device, and sends `acknowledged: true` once it is ticked.

### Settings → AI providers

Each provider is a card with its brand mark, privacy posture, kind (HTTP or
CLI agent), and default badge. CLI agents sign in through their own CLI, so
their cards show a runtime status instead of key storage: installed version
and signed in, not installed, sign-in needed, or an error, with the broker's
sanitized hint (for example ``Run `copilot login` outside Kassiber.``) and
when it was checked. HTTP cards show the base URL with the API format the next
chat speaks (the pinned format, the detected one, or that auto has detected
nothing yet), key storage, and whether the last check reached the endpoint and
how many models it listed. A provider set to `omit_store_field` says on its
card that it sends requests without Kassiber's no-retention flag and that the
provider's own retention applies.

Opening the page contacts no provider. Cards read the daemon's last result
with `cached_only: true`, which never starts a probe, even on a cold cache:
`ai.provider_runtime.status` for a CLI agent, `ai.list_models` for an HTTP
provider. The daemon keeps each provider's last check in the book
(`ai_provider_checks`), so after a restart a card still shows what it found
and when ("checked 3 days ago") instead of "Not checked yet". Only a check the
user started writes it: **Check**, **Check all**, **Set up**, or the picker's
**Check models**. It is read back only while the provider has the base URL it
was made at, and a new key or any edit that clears the discovery cache
deletes it; a later check that fails reports it marked stale. `cached_only`
with `refresh` is refused. `ai.providers.list` also reports each provider's
`last_checked_at` and `set_up`.

Built-in providers the user has not used yet are folded into a collapsed
**Not set up** group: a built-in row still at its built-in address and name
that is not the default, was never checked, and (off-device) was never
acknowledged. Each row has **Set up**, which switches it on if needed and runs
its check, asking for the acknowledgement first as any check does, and a
remove button. The group is display only; it never decides whether a provider
may be contacted, and the model picker still lists every enabled provider.
Providers the user added are always shown as cards. CLI cards
also show a **Check models** result from the picker: when it is newer than
the last runtime check, the card lists its models (and fast mode where one
offers it) as "Models listed", keeping a ready runtime's version line. A
newer runtime check that is not ready shows no models. Results
belong to the base URL that was checked (for a CLI agent, the runtime its
locator names), in the desktop and in the daemon's discovery cache, so after
the URL is edited the card reads "not checked", or shows the new runtime's
last cached result, until the user checks it again; nothing probes the new
endpoint on its own. A check is bound to the endpoint the user chose: the
desktop sends that `base_url` with `ai.list_models` and the scoped
`ai.provider_runtime.status`, and if the row now names a different one
(edited in Settings or from the CLI while the check waited for the
acknowledgement), the daemon refuses with `ai_provider_changed` instead of
contacting the new endpoint. Callers that omit `base_url` check whatever the
row holds. **Test connection** in the provider form needs no such binding,
since it names the endpoint it tests (see [Connection test](#connection-test)). **Check** asks one provider now:
`ai.provider_runtime.status` scoped with `provider` for a CLI agent, which
starts only that CLI, or the picker's `ai.list_models` refresh for an HTTP
provider, so either check also fills the picker. A CLI check that finds the
agent not installed, signed out, or failing clears the picker's model list
for it, so models and capabilities such as fast mode are no longer offered
from an earlier check. **Check all** does the same
for every enabled provider, one request per provider. Off-device providers
need the acknowledgement first (one confirmation for all of them); if it
cannot be recorded, each of those cards shows why and none of them is
checked, while the other providers still are. Offline
mode refuses them before anything is asked, and the checks are unavailable
while AI features are off. A failed check is reported once rather than
retried, and its error belongs to that book and endpoint like the results
do. A check sends no prompt.

Each card also carries:

- **An enable switch.** A disabled provider is hidden from the model picker
  (a remembered selection on it falls back to the default provider) and is
  never contacted: `ai.chat`, model listing (`ai.list_models` and
  `kassiber ai models`), its runtime check, `ai.test_connection` for that
  saved provider, and the photo/PDF importer refuse it with
  `ai_provider_disabled` before anything is sent, and its **Check** button is
  unavailable. The unscoped `ai.provider_runtime.status` form starts only a
  CLI that has at least one provider row that is enabled and acknowledged
  for off-device use, the same gate as the per-provider check, so a seeded
  or switched-off CLI is never started by it, not even on a cold cache. It
  sends the broker one status request naming just those runtimes, which are
  probed in parallel under the same single 35-second timeout; when none
  qualify it starts nothing. The stored default provider cannot
  be disabled, and a disabled provider cannot become the default. Earlier
  check results stay visible across switch and default-effort edits.
- **A default reasoning effort** (`low`, `medium`, `high`, `xhigh`, `max`, or
  `ultra`; empty means auto; an HTTP provider offers only the first three). It
  seeds a session's effort when that provider
  becomes selected in the desktop or `kassiber chat` starts with it, but only
  an effort the user left open: auto, or one that came from the previous
  provider's default. A level the user picked in the desktop, or
  `--reasoning-effort`, survives provider switches; in the desktop, if the
  new provider's model does not offer it, that provider's default applies,
  else the nearest offered level. The same fallback runs when **Check
  models** later reveals that the selected model does not offer the current
  level, rather than the picker dropping it to auto; a level the model offers
  is left alone. Provider settings belong to a book, so
  opening another book seeds the effort again from that book's provider row,
  even for a provider of the same name, and a level chosen in the previous
  book does not carry over. Until the open book's provider list has loaded
  and seeded the effort (at launch, or briefly after a book switch), the
  composer holds sending and its send button says why, and a prompt sent
  meanwhile, including the regeneration of an edited message, waits in the
  queue, so none goes out with the previous book's effort or with auto in
  place of a default.
  If that list fails to load, sending resumes with auto (or a level the
  user picked in the new book), never the previous book's level.
  The provider still decides whether it honours
  the level, as with a per-turn choice. Over HTTP, `xhigh`, `max`, and
  `ultra` are sent as `high` (see [Reasoning effort over
  HTTP](#reasoning-effort-over-http)).
- **A fast mode note** where one of the provider's models offers fast mode.
  Fast mode has no stored default: it is billed at a higher rate, so it stays
  a per-session choice in the reasoning menu.

If Kassiber itself is running inside a container and Ollama is running on the
host, seed the provider with the Docker host alias instead:

```bash
KASSIBER_DEFAULT_AI_BASE_URL=http://host.docker.internal:11434/v1
```

This only affects first-time provider seeding. For an existing book, update the
`ollama` provider's `base_url` instead.

To make oMLX the default for a brand-new book, set:

```bash
KASSIBER_DEFAULT_AI_PROVIDER=omlx
KASSIBER_DEFAULT_AI_BASE_URL=http://127.0.0.1:8000/v1
```

Per-provider seed overrides are also supported:

```bash
KASSIBER_OMLX_AI_BASE_URL=http://127.0.0.1:8000/v1
KASSIBER_OLLAMA_AI_BASE_URL=http://host.docker.internal:11434/v1
```

Choose a model that supports the required tool or image input contract and
verify it against the intended workflow. The photo/PDF transaction importer
requires a local loopback provider and an installed vision/OCR model. Remote,
TEE, and CLI providers are hard-disabled for that path.

The CLI providers appear automatically through fixed provider locators:

```bash
codex-cli://default
claude-cli://default
opencode-cli://default
copilot-cli://default
```

No Settings row or API-token entry is required. The broker discovers installed
executables, reports `ready`, `missing executable`, or `authentication
required`, and tells the user to run the provider's normal login command
outside Kassiber when necessary. It uses Codex `app-server`, the Claude
executable's `--output-format stream-json` event stream with strict MCP config,
and the OpenCode SDK v2 against an ephemeral loopback OpenCode server, and the
Agent Client Protocol for GitHub Copilot. A local Node.js executable
meeting the [broker package requirements](../../ui-tauri/package.json) is
required to run the bundled broker.

### Agent Client Protocol agents

[ACP](https://agentclientprotocol.com) is JSON-RPC over an agent CLI's stdio.
One generic adapter (`ui-tauri/provider-broker/src/acp.ts`) carries the
conversation, permission requests, and the Kassiber MCP server for every agent
in its table, so adding an agent is a table entry rather than a new protocol
client. ACP has no portable way to switch off an agent's own shell, file, and
web tools, and agents run some tools without asking. An agent is therefore
listed only when its CLI can be started with all of them removed and with none
of the user's MCP servers, plugins, hooks, memory, or settings loaded; that
launch is the boundary.

**GitHub Copilot** (`copilot --acp`) is the one agent that meets it today:

- `--available-tools` lists only the advertised `kassiber-<tool>` names, or a
  name that matches nothing, so the model sees no shell, file, web, task, or
  skill tool. Copilot applies that list in ACP mode from 1.0.60 on, so status
  and chat refuse an older CLI, or one whose version cannot be read, before
  starting it. `--disable-builtin-mcps` and `--no-custom-instructions` drop
  GitHub's servers and repository instructions.
- `COPILOT_HOME` points at an empty 0700 directory inside the turn's working
  directory. The user's `mcp-config.json`, plugins, settings (including a
  persisted allow-all), and saved sessions therefore never load. Copilot
  cannot be told not to save the session, so it writes this turn's
  conversation and tool results to a plaintext session file in that
  directory; Kassiber waits for Copilot to exit and deletes the directory when
  the turn ends, but a crash mid-turn leaves it behind. On Linux the turn's
  directory is under `$XDG_RUNTIME_DIR` when that is a tmpfs owned by the user
  and writable by no one else, so the file stays in RAM; otherwise, and on
  macOS and Windows, it is in the disk-backed system temporary directory.
  Sign-in still comes from the system keyring or `COPILOT_GITHUB_TOKEN` /
  `GH_TOKEN`; a login kept only in the user's own Copilot config is not
  found, and status reports a sign-in problem.
- Copilot ignores MCP servers sent in `session/new`, so Kassiber's server goes
  through `--additional-mcp-config`. `COPILOT_ALLOW_ALL` and
  `COPILOT_PROVIDER_API_KEY_COMMAND` (a shell command Copilot would run on
  every request) never reach the CLI; the other bring-your-own-model settings
  are passed by name.
- Because the session lives in that per-turn home, each turn starts a new
  Copilot session from the visible transcript; there is no resume cursor.

Each ACP session also fails closed behind that launch. Kassiber advertises no
client file system or terminal. A tool call or permission request is accepted
only when its title is exactly the agent's form for an advertised Kassiber tool
and it is not an `execute` call, and a later update to that call may not
change it into another tool or into execution; anything else is rejected and
stops the agent. A shell tool's title can be its model-chosen command line, so the title
check is a tripwire rather than the boundary. Selected-data (sensitive context)
requests are refused. Startup notices an agent sends as message text are
dropped. The handshake must finish within 25 seconds so the broker can still
stop the agent before the daemon's 35-second status deadline, and a probe that
overruns is ended with its whole process group.

Agents that speak ACP but are not listed:

- **Gemini CLI** (0.63.0): its system settings file is the only layer that
  outranks user settings, and Gemini ignores one that is not owned by root, so
  Kassiber cannot apply it. User `tools.discoveryCommand` /
  `tools.callCommand` entries would also run regardless of `tools.core`, and
  `~/.gemini/GEMINI.md` memory is sent to the model.
- **Cursor** and **Grok**: neither can remove its built-in tools in ACP mode.

Model and reasoning-effort selection are forwarded through each provider's
native protocol.

**Fast mode** is a faster serving mode billed at a higher rate. Codex models
offer it on Codex 0.151.0 and later (read from the `initialize` user agent)
when their `model/list` entry carries the Fast service tier. A fast turn
resolves the model the thread actually runs (including Kassiber's `default`),
checks that tier on the chat's own app-server connection, and only then sends
the tier id the catalog advertises (usually `priority`, or the legacy `fast`
when an entry lists only `additionalSpeedTiers`) as `serviceTierForTurn`;
every other turn asks for `"default"` explicitly, so a saved
`service_tier = "fast"` in the user's Codex config or an earlier fast turn on a
resumed thread cannot keep billing fast. `serviceTierForTurn` leaves the
thread's saved tier alone, so a Kassiber thread continued in Codex itself never
inherits fast. Older app-servers ignore that field, and their only tier field,
`serviceTier`, would also become the thread's tier, so there fast mode fails
closed: it is not offered, a fast request is declined with the notice below,
and the turn sends `serviceTier: "default"`. App-servers from before the
`"default"` id took `null` for standard speed and reject `"default"` while
deserializing, so a rejected `"default"` is retried once as `null` and ordinary
chat keeps working. Claude offers it on the `opus` alias only, through the
inline `--settings '{"fastMode":true}'`, and only when status reads Claude Code
2.1.205 or later, the first CLI that honours that setting under `-p`; other
aliases accept the setting but ignore it. Every other Claude turn passes
`{"fastMode":false}`, so a resumed native session that an earlier turn ran fast
(or a `default` alias resolving to Opus) does not stay fast once Fast is off.
With fast mode on, Claude Code also checks its availability directly with
`api.anthropic.com`, outside `ANTHROPIC_BASE_URL`; Kassiber turns fast mode on
only for a turn the user sent with Fast on (see
[privacy and security](privacy-and-security.md#external-requests-complete-list)).
When fast mode cannot apply (an
unsupported Codex model or version, or Claude's last reported
`fast_mode_state` before its result being other than `on`, as after a
rate-limit `cooldown` partway through, or missing, as from a CLI older than
fast mode; or Claude's `system`/`notification` that it retried the request at
standard speed after usage credits ran out, which leaves the state `on` and is
recognised by its fixed key or its leading "Fast mode disabled", never
forwarded), the turn continues at
standard speed and the daemon emits an `ai.chat.status` with phase
`fast_mode_unavailable`, which the desktop shows under the answer and
`kassiber chat` prints. The answer keeps it too: the terminal `ai.chat`
provenance lists it in `notices` (so `kassiber chat --machine` returns it), and
a stored assistant message keeps that provenance, from which the desktop shows
the notice again when a saved chat is reopened, including in a branched or
edited chat that copied the answer (see `seed_notices` below). Only
recognised notice phases are listed, never provider text: this one and
`reasoning_effort_unavailable` (see [Reasoning effort over
HTTP](#reasoning-effort-over-http)). Copilot, OpenCode
and HTTP providers have no fast mode, and no provider stores it as a default.
The option (`options.fast_mode`, a boolean) crosses to the provider broker only
when it is `true` and is never put in an HTTP request body. The desktop offers
the toggle in the reasoning menu only for a model whose checked row advertises
it; that row depends on the installed CLI's version, so nothing is offered
before Check models (which never runs on its own). It drops the toggle when the
selected model changes to one that does not offer it;
`kassiber chat --fast` requests it from the CLI.

Model discovery follows each provider's protocol. Where discovery supplies
aliases, forward those aliases rather than maintaining versioned model IDs in
this document. The provider's configured default applies when no model is sent.
Provider session cursors are kept only in daemon memory and
are reused only while the visible Kassiber transcript remains unchanged;
editing or branching starts a clean native provider session.

## In-app surface

The desktop assistant is a tab of the right panel beside every
authenticated screen, next to an open transaction's details, with a composer
or launcher below the page while it is not in the panel (see
[the desktop reference](desktop.md)). Its provider/model picker is fed by
`ai.providers.list` and `ai.list_models` over the daemon protocol; chat
streaming is wired through Tauri events (`daemon://stream`) so the UI can
render loading status, reasoning (`<think>`), and the answer in real time
without blocking navigation.

The picker uses a provider rail plus a searchable model list. **Check models**
discovers only the selected provider through `ai.list_models`; checking one
native CLI must not refresh global runtime status or start other providers.
Stored configuration and cached models remain visible without discovery.
First use still requires Kassiber's off-device acknowledgement.

Typing in the picker ranks every provider's already-known models (configured
defaults and earlier check results) at once; it never contacts a provider.
Starred models collect under **Favorites** at the top of the rail. Favorites
are stored with the UI preferences as bare provider/model identifiers, like
the current selection. The picker is keyboard-driven: Mod+Shift+M opens it
from the composer, the arrow keys and Enter pick a row, Mod+1…9 pick the nth
row, Mod+D stars or unstars the highlighted row, and Mod+Shift+Up/Down switch
provider. Keys that confirm an input-method composition never pick or star a
model. The collapsed trigger shows the provider name, model and privacy
posture.

The **All / Local** control filters the inventory by the posture Kassiber can
prove. Model rows show both their source namespace and privacy posture.
OpenCode model IDs identify their source provider (`omlx/...`, `ollama/...`,
`opencode/...`). OpenCode's model list carries no endpoint — `api.url` is empty
for locally served models — so the route is judged by that source provider's
resolved endpoint from `opencode debug config`, reading
`provider.<id>.options.baseURL` and nothing else (the same options block can
hold an `apiKey`, which never leaves the broker). A loopback endpoint (`127.0.0.0/8` matched as a literal IPv4
address, `localhost`, `::1`) is labelled local, because the model cannot leave
the machine. A loopback address that fronts a reverse proxy is the known
limit of this check: it reports local while the proxy may forward inference
off-machine. Everything else stays remote, including LAN addresses and
any provider for which OpenCode reports no endpoint. The posture is proven from
configuration, never guessed from a provider's name.

An HTTP provider's models that cannot run the assistant are hidden by default.
That is a model whose cached listing says it takes no tools
(`supports_tools: false`) or lists output modalities without `text`, and, where
a listing says neither (as OpenAI's never does), only a model whose id carries
a clear non-chat mark: `embed`, `whisper`, `dall-e`, `gpt-image`, `moderation`,
`transcribe` or `realtime` anywhere, or `tts` or `audio` as a separate word
(`tts-1`, `gpt-4o-mini-tts`, `gpt-audio`). A bar under the list counts what is
hidden, including in search results, and offers **Show all models**. The
selected, default and favorite models always stay listed, CLI agents' models
are not filtered, and a provider without a default model starts on its first
usable one. If none of its listed models is usable, nothing is selected and
the picker says so ("No model from this provider can run the assistant."), and
**Show all models** still lets the user pick one deliberately. The filter reads only the cached listing and never asks a
provider.

Provider and model discovery uses a daemon-owned, in-memory last-good cache.
Snapshots include `checked_at`, `stale`, and sanitized `error` metadata, and
concurrent refreshes for the same provider share one probe. Failed refreshes
keep the last successful inventory visible. Opening an AI surface or its model
picker does not contact any provider, including loopback HTTP services and
native CLIs. Refresh requires **Check models** for that provider; testing a
provider in Settings is also an explicit action. Provider configuration changes
invalidate the daemon cache but do not authorize a probe.

Assistant answers, reasoning and restored transcripts render Markdown image
references as inert buttons, with no image or preload resource. Opening an image
requires an explicit click and uses the existing external-browser action; only
absolute HTTP(S) URLs without embedded credentials are accepted. Unsupported
image references remain text. This applies in the native app and the supported
browser development UI, independently of the native Content Security Policy.

Kassiber's HTTP AI client ignores operating-system and environment proxy
settings and rejects redirects outside the configured provider origin. Route a
remote HTTP provider through its explicitly configured base URL. Native CLI
providers retain their own executable transport settings, as described below.

Each broker probe or chat runs from a fresh Kassiber-owned empty temporary
directory that is removed afterward. The daemon gives each broker a temporary
root of its own and holds those working directories there, so it can remove
them once the broker has exited, however it exited. A broker ended with SIGTERM
also stops its provider processes and removes its tool bridge first; a probe
that overruns its deadline gets SIGTERM, then SIGKILL for its whole process
group. On Windows, terminating the broker does not yet end the provider
processes it started. Provider subprocesses receive only their
own authentication/configuration environment plus the shared network/runtime
minimum; unrelated provider and Kassiber secrets are excluded. Claude gets an
empty built-in tool set, no filesystem setting sources, the file/exec/network
tools named in `--disallowed-tools`, strict MCP config, and an allow-list of
only the advertised `mcp__kassiber__*` names; OpenCode gets a deny-all
permission ruleset with exact allows for the temporary Kassiber MCP tools,
and sharing switched off on every turn (a share uploads the session to a
public link). Kassiber's inline config sets `share: "disabled"` over the
user's, `OPENCODE_DISABLE_SHARE=1` turns the share service off,
`OPENCODE_AUTO_SHARE` never reaches the CLI, and a turn whose effective
config still enables sharing (a managed or organisation layer merged later)
stops before any session is opened. A user `OPENCODE_CONFIG_CONTENT` must be
plain JSON so Kassiber can merge into it. Codex runs read-only and untrusted with network access disabled and
capability-scoped `dynamicTools`. A tool request outside the advertised
Kassiber catalog aborts the turn for Claude, OpenCode, and Copilot;
Codex returns a
failed tool result instead, so its turn continues without the tool. The broker receives no repository, database, attachment, wallet,
browser, terminal, or source-control capability, and no ability to widen its
own catalog: it forwards a tool call to the daemon and waits.

Claude, OpenCode, and Copilot reach Kassiber through a child MCP
process, and the bridge
carrying its calls is a unix socket inside a private 0700 directory removed
with the turn. A loopback TCP port would be reachable by every local process
and would need a shared secret; the only place to hand one to the child is
`argv`, which is world-readable on Linux, so filesystem permissions replace the
secret. Codex needs no child process — its adapter answers `item/tool/call`
in-process and never touches the socket. On Windows the bridge is a named pipe
with an unguessable name in the global pipe namespace rather than a file in the
private directory, so it is protected by the default per-logon pipe security
descriptor, not by directory permissions. In every case the boundary is other
operating-system accounts; a process already running as the desktop user can
reach Kassiber's data by many other routes and is not defended against here.

Prompts, answers, and the tool results that reach these providers also land in
their own plaintext session stores — Codex threads, Claude sessions, OpenCode
sessions — which Kassiber neither encrypts nor prunes. Resuming a chat depends
on those stores, so the accounting context of a turn persists outside
SQLCipher until the provider's own retention removes it, whatever Kassiber's
history setting says. Copilot's store lives in the turn's working directory
and is deleted with it, so only an interrupted turn leaves one behind.
Selected-data turns run Codex, Claude, and OpenCode without session
persistence, and Copilot refuses them.

Incognito turns do the same for those three while keeping their tools. The
daemon turns `persist: false` into the broker option `stateless_session:
true`. The broker starts an ephemeral Codex thread (also with history
persistence off), runs Claude with `--no-session-persistence`, and points
OpenCode's database at memory and its data and state homes at the turn's
directory. If the CLI cannot comply (Codex does not report the thread as
ephemeral, Claude's help lacks the flag, or OpenCode reports a database on
disk), the turn fails before the prompt is sent. On Windows, OpenCode
Incognito turns are refused before OpenCode starts. Keeping OpenCode's log
file off disk needs a file symlink to the null device, which a normal
Windows account cannot create, and OpenCode cannot switch the file off; the
desktop's Incognito tooltip says so on Windows. A stateless turn neither
sends nor returns a provider session cursor. The next turn starts a new
native session from the visible transcript. Copilot is not refused. Its
session file still exists for the turn in the per-turn directory, and the
desktop's Incognito tooltip and `kassiber chat --incognito` say so. HTTP
providers ignore the option: their requests already send `store: false`, over
the Responses API and Chat Completions alike, and the option never reaches
their request body.
Local HTTP providers avoid the provider-side session stores, but Kassiber
cannot control what a local server logs itself, so the tooltip promises only
what Kassiber keeps. See
[privacy and security](privacy-and-security.md#local-storage).

For browser-driven development, the Vite dev server also exposes a loopback-only
daemon bridge. Run:

```bash
pnpm --dir ui-tauri run dev:bridge
```

Then open `http://127.0.0.1:5173`. In bridge mode, the browser talks to the
same local Python daemon protocol through Vite: normal calls use
`/__kassiber__/daemon`, and `ai.chat` streams NDJSON records from
`/__kassiber__/daemon/stream`. This is a development-only convenience for
testing real local AI, tool cards, cancellation, and consent from an ordinary
browser tab. Packaged builds and `tauri dev` do not use the bridge.

Provider configuration is mirrored in the CLI:

```bash
kassiber ai providers list
printf '%s\n' "$OPENAI_API_KEY" | kassiber ai providers create openai --base-url https://api.openai.com/v1 --acknowledge --api-key-stdin --default-model gpt-4o-mini
printf '%s\n' "$OPENROUTER_API_KEY" | kassiber ai providers create openrouter --base-url https://openrouter.ai/api/v1 --api-format chat-completions --acknowledge --api-key-stdin --default-model openrouter/auto
kassiber ai providers create lmstudio --base-url http://localhost:1234/v1 --default-model local-model
kassiber ai providers test lmstudio
kassiber ai providers create claude-cli --base-url claude-cli://default --acknowledge --default-model default
kassiber ai providers set-default openai
kassiber ai providers update codex --disable
kassiber ai providers update claude --enable --default-effort high
kassiber ai models
kassiber chat "Summarise the last week of imports."
git log --oneline -20 | kassiber chat -
kassiber chat
```

Without `--kind`, `ai providers create` makes a loopback URL `local` and
anything else, CLI agents included, `remote`; `tee` is never inferred, and a
remote provider still needs `--acknowledge` before anything is sent to it. An
update changes the kind only when asked, except that a local provider given a
non-loopback `--base-url` becomes remote; leaving local, either way, needs
`--acknowledge` again, since a local provider's implicit acknowledgement never
covered sending prompts off the device. A remote or TEE provider moved to a
loopback URL stays off-device, because a loopback gateway can still forward
prompts. OpenRouter's main API is Chat Completions (its Responses API is
stateless and newer), hence the pinned format above; with `auto`, the first
chat or `ai providers test` finds a format by itself.

`kassiber chat` is the CLI client for the same daemon-backed assistant used by
the desktop UI — there is exactly one chat surface and one protocol. It starts
a local daemon transport, sends `ai.chat` requests with `tools_enabled=true`,
renders streaming deltas in the terminal, and sends `ai.tool_call.consent`
decisions when mutating tools ask for approval. Pass a prompt positionally or
with `--prompt` for one turn; `kassiber chat -` reads the one-shot prompt from
stdin for pipelines and heredocs. After each rendered turn a dim provenance
footer shows provider/model, the tools that actually ran, and whether journals
were auto-refreshed — the same provenance the desktop Assistant records.
`--fast` asks for the model's fast serving mode where it has one (see above).
`--no-tools` disables the tool loop for a provider-only exchange, and
`--system "..."` replaces the built-in Kassiber system prompt with a raw one
(`system_prompt_kind="raw"`).

Use `--tool-profile` to select the exposed catalog; defaults and scope behavior
are documented under [Tool use](#tool-use).

Use `--timeout SECONDS` for harnesses or local models that need a shorter or
longer wait. It caps daemon startup and provider stream inactivity (default
120 seconds). The value is local transport control only: Kassiber does not put
it in system prompts, user messages, tool schemas, tool results, or transcripts.

Omit the prompt for REPL mode, which has line editing and in-session history
on real terminals, and these commands:

- `/help` — command help; `/exit`, `/quit`, or Ctrl-D leaves.
- `/tools` — the daemon tool catalog with consent classes.
- `/model [id]`, `/provider [name]` — show or switch mid-session; a provider
  switch re-resolves that provider's default model and rolls back on error.
- `/allow <tool>`, `/allowed` — manage which mutating tools are pre-approved
  for this session.
- `/new` — start a fresh conversation without restarting the daemon.

Ctrl-C during a reply cancels that turn cooperatively and keeps the session.
Daemon-side `allow_session` consent spans a single `ai.chat` request, so the
REPL carries an interactive "[s] session" answer across turns client-side and
re-sends it for that tool.

Output modes cover scripting:

- default rendered text for humans. On a TTY, the model's markdown is
  rendered with ANSI styling — bold, inline code, headers, bullets, fenced
  code, and pipe tables re-drawn as box-aligned tables — while preserving
  token streaming, and successful tool results draw a compact deterministic
  table straight from the daemon envelope, so tabular numbers on screen never
  depend on the model retyping them correctly. `--plain` turns both off.
  With piped stdout, the raw answer text is the only thing on stdout —
  progress labels, tool announcements, tool tables, consent UI, and the
  provenance footer move to stderr;
- `--machine` / `--format json` (one-shot only) emits a single `chat` envelope
  with the final message, `finish_reason`, provenance, and tool-call summary;
- `--stream-json` (one-shot only, mutually exclusive with `--machine`) emits
  the raw daemon stream records — `ai.chat.status`, `ai.chat.delta`,
  `ai.chat.tool_call`, `ai.chat.tool_consent_required`, `ai.chat.tool_result`,
  then the terminal `ai.chat` — as NDJSON, mirroring what the desktop bridge
  streams;
- `--transcript PATH` (any mode, REPL included) appends every daemon request
  and stream record for the session to PATH as NDJSON — a local audit trail
  for debugging model answers and tool behavior. The file is plaintext and
  contains prompts and redacted tool results; treat it like notes, not like
  the encrypted database.

For automation, `kassiber chat --yes "..."` approves mutating tool requests for
that chat session without prompting, except `ui.review.apply`: each custody
review requires a fresh interactive answer to the daemon-validated preview.
Neither `--allow-tool`, `/allow`, nor prior session consent bypasses that
requirement, and non-interactive chat denies this tool. Use the explicit
`review plan/apply` CLI for externally reviewed artifacts. Prefer the narrower
`--allow-tool ui.journals.process` form when a script should approve only one
tool. Machine and `--stream-json` runs never prompt interactively even on a
TTY; there, and without a TTY in rendered mode, unapproved mutating tools are
denied and the denial is fed back to the model as `user_denied`.

If the local database is SQLCipher-encrypted and the CLI cannot unlock it,
`kassiber chat` fails with `passphrase_required` before contacting the model.
Headless scripts should pass the global `--db-passphrase-fd <FD>` flag from a
parent process. The passphrase is consumed by the local daemon only; it is not
embedded in prompts or tool output.

Test the assistant/tool path through the CLI, not only through the desktop GUI:
`tests/test_cli_chat.py` pins the daemon-backed chat loop, consent behavior,
locked-database handling, timeout controls, and the tool profiles.

Tool-profile selection is described under [Tool use](#tool-use).
Live backend checks should also be CLI-first, with explicit user-approved
endpoints and a fresh temporary data root.

## Chat history

Chat sessions can persist — inside the SQLite/SQLCipher database, next to the
data the answers were derived from, never as separate plaintext files. The
policy setting `ai_chat_history` has three values, managed via
`kassiber chats config [--history auto|on|off]`:

- `auto` (default) — persist only when the database file is
  SQLCipher-encrypted. A plaintext database stays ephemeral; running
  `kassiber secrets init` is what unlocks history.
- `on` — persist regardless of encryption (an explicit user choice).
- `off` — never persist.

`kassiber chat --incognito` skips persistence for one session regardless of
the setting. History `off` governs Kassiber's own history only: a CLI agent
provider still keeps the conversation in its own session store. Incognito
also asks Codex, Claude, and OpenCode not to save it, and states what GitHub
Copilot still keeps for the turn (see [In-app surface](#in-app-surface)).
`kassiber chat --continue` resumes the most recently updated
session (the stored messages are replayed to the model as context);
`--session <id>` resumes a specific one. In the REPL, `/new` starts a fresh
session. Stored exchanges keep the user prompt, the assistant answer, the
`finish_reason`, and the answer provenance — not full tool result envelopes,
which remain reproducible from the database (use `--transcript` for
full-fidelity capture).

Answer provenance includes a UI-only `privacy_receipt`: provider kind,
local/remote classification, screen route, number of advertised schemas and
executed tools, plus outbound event/endpoint/byte counts recorded during that
turn. The receipt is computed after the provider call and is never fed back to
the model. Exact hosts remain available only on the dedicated local Egress
screen; the AI-facing egress tool receives aggregate subsystem counts.

Manage stored sessions with `kassiber chats list`, `chats show <id>`,
`chats delete <id>`, and `chats clear`. Machine chat envelopes and the
terminal `ai.chat` record carry `session_id` (null when nothing persisted).

On the wire, persistence is per request: `ai.chat` accepts
`persist: true | false | "auto"` plus `session_id` to append to an existing
session. A session id that is not a saved chat of the open book fails with
`not_found` before the provider is resolved, whether or not the turn is
stored. `persist: false` is Incognito: it also sets `options.stateless_session`
for a CLI provider (see [In-app surface](#in-app-surface)). A request opts in
by sending `persist` true/`"auto"` or a `session_id`; with neither, nothing
persists, so existing clients are unchanged. The stored policy stays
authoritative over every write — `off` never persists and `auto` persists
only on encrypted databases, even for continuations of an existing session.
A branch or edit sends `seed_history: true` on its first turn, so the new
session stores the prior on-screen turns as its seed, plus `seed_notices`
(`[{index, notices}]`, by `messages` index) for the copied answers that carried
a notice. The daemon accepts only assistant messages before the current prompt
and keeps only recognised notice phases, stores them as those messages'
provenance, and never adds them to the model's messages. Session management is exposed as the daemon
kinds `ui.chat.sessions.list`, `ui.chat.sessions.get`,
`ui.chat.sessions.delete`, and `ui.chat.sessions.clear`, profile-scoped like
the rest of the UI surface, plus `ui.chat.history.configure` for reading or
setting the policy (the desktop Settings control). The desktop Assistant
sends `persist: "auto"`, round-trips `session_id`, and surfaces history in
its toolbar; these kinds stay usable while the AI runtime toggle is off,
because seeing and deleting stored history is a privacy control, not an AI
feature. Chat history is intentionally **not** an AI tool:
the model cannot browse or search prior sessions on its own. Only a session
the user explicitly resumes is replayed as normal chat context, so
prompt-injection risk stays scoped to the resumed conversation. Diagnostics
reports and audit packages do not include chat content.

Provider API-key entry supports `--api-key-stdin` and `--api-key-fd FD`. The
legacy `--api-key <value>` form still works as a warning-on-use compatibility
shim, but docs and tests avoid it because argv can land in shell history and
process listings. Desktop Settings uses the narrow `ai.providers.set_api_key`
daemon kind to rotate/re-enter a key and `ai.providers.move_api_key` to move a
stored key between `sqlcipher_inline` and the selected native store. The daemon
rejects `api_key` on `ai.providers.create` and `ai.providers.update`.
`ai.test_connection` accepts a key typed into the form for that one test and
never stores it; without one it reuses the saved provider's key, but only for
the saved provider's own base URL.

Provider envelopes expose only `has_api_key` plus
`secret_ref.{store_id,state}`. `sqlcipher_inline` keeps the key in the
SQLCipher database. Native desktop storage records only non-secret
`ai_provider_secret_refs` metadata and stores the value in macOS Keychain,
Windows user-scope Credential Manager/DPAPI, or Linux Secret Service when
platform policy selects that store. Backup export records only ref metadata for
non-inline AI keys and refuses inconsistent rows where a non-inline ref still
has an inline `api_key`; backup import surfaces a non-fatal
`secret_ref_unavailable` warning so Settings can prompt for re-entry. Backend
tokens, descriptors, xpubs, and blinding keys stay SQLCipher-protected. See
[`../plan/10-secret-management.md`](../plan/10-secret-management.md).

Reasoning-capable models surface provider-exposed reasoning summaries or
model-authored thinking text through one of two channels, and both are split
into collapsible reasoning pane(s) above the answer:

- Inline `<think>...</think>` tags inside the content stream — emitted by
  DeepSeek-R1 and QwQ.
- Structured reasoning: Responses reasoning text or summary events, which
  Ollama/oMLX thinking models stream without being asked (Kassiber does not
  request OpenAI's summaries, see [Reasoning effort over
  HTTP](#reasoning-effort-over-http)), and the Chat Completions
  `reasoning_content` or `reasoning` fields.

Each user turn gets its own assistant message. Inside a tool-using turn,
each provider completion round (`waiting_for_model` before the next model
call) opens a fresh reasoning segment, so Ollama/oMLX traces stay
per-round instead of one continuous blob. Models that don't emit either
channel pass through unchanged.

Settings → AI providers exposes a **Test connection** action. It calls the
daemon's `ai.test_connection` kind with the *currently entered* base URL and
API key (or, when editing without changing the API-key field, the saved key)
and reports the model count. It also passes the form's privacy kind (`kind`),
since in [offline mode](privacy-and-security.md#offline-mode) only a provider
marked local may be tested; without it the saved provider's kind applies. For
built-in CLI locators, model discovery and authentication readiness come from
the broker's native runtime probes. For HTTP providers the test also sends the
fixed request described under [Connection test](#connection-test), which spends
a few tokens, detects the API format and whether tools are accepted, and stores
only that detection, only on the saved provider whose endpoint it tested.

Remote, TEE, and CLI providers require explicit
acknowledgement before chat. The CLI uses
`kassiber ai providers update <name> --acknowledge` (or `--acknowledge` during
`create`), and the desktop Settings form prompts before saving an off-device
provider. Without that acknowledgement, `ai.chat` returns
`ai_remote_ack_required` before sending any prompt content. A provider switched
off with `--disable` (or in Settings) returns `ai_provider_disabled` the same
way.

Streaming is demuxed by `request_id`: the Tauri supervisor keeps one daemon
process and one stdout reader, but routes each JSON envelope to the matching
request. While a chat is streaming, unrelated daemon calls can complete
independently.

Before the first token arrives, `ai.chat` may emit `ai.chat.status` records
with phases such as `preparing`, `connecting`, and `waiting_for_model`. These
records are UI progress hints only; model-authored thinking is shown only when
the provider emits inline `<think>` content or structured reasoning-summary
deltas.

Pressing **Stop** in the desktop UI, choosing cancel at a terminal consent
prompt, or interrupting `kassiber chat` sends `ai.chat.cancel` with
`args.target_request_id = <active ai.chat request_id>`. Cancellation is
best-effort and cooperative: Kassiber stops forwarding deltas once the Python
worker returns between provider chunks, then emits the terminal `ai.chat`
envelope with `finish_reason: "cancelled"`. For metered remote providers, any
tokens already generated or in flight may still be billed.

## Tool use

External agents can reach a read-only, redacted slice of this catalog through
`kassiber mcp serve`; that allowlist, its disabled read maintenance, and its
book pinning are documented in [MCP](mcp.md).

The desktop assistant and `kassiber chat` opt into a bounded tool loop with
`ai.chat` top-level args:

```json
{
  "tools_enabled": true,
  "tool_loop_max_iterations": 8,
  "system_prompt_kind": "kassiber"
}
```

Tool control stays top-level; generation options still live under `options`.
When enabled, Kassiber prepends a compact Kassiber skill-aware system prompt,
sends the function definitions in the provider's format (flat Responses
functions, or nested Chat Completions functions), emits `ai.chat.tool_call`,
`ai.chat.tool_consent_required`, and
`ai.chat.tool_result` stream records as needed, feeds tool results back as
typed `function_call_output` Items with matching `call_id` values (`tool`
messages with `tool_call_id` over Chat Completions), and finishes with the
normal terminal `ai.chat` envelope. The provider's output is replayed between
tool rounds so its reasoning is kept where the endpoint allows (see
[Stateless requests](#stateless-requests)).

Before the provider is called, Kassiber also runs a small deterministic
read-only router for Kassiber questions. It looks for common accounting intents
such as pending work, sync readiness, totals, inflow/outflow, balances, tax
summaries, largest/smallest transactions, transaction search, quarantine,
the combined review worklist, loans, book-set views, transfers/direct payouts,
swap-review context, saved review filters, auto-pair rules, and pricing.
Matching read-only tool results are streamed to the UI and inserted into the
model context as exact local data, so answers use program-derived facts. That auto-read context is sent as
untrusted accounting data, not as system instructions, and is bounded per tool
so large reports cannot silently crowd out everything else.
When one of those reads needs current reports or journal-derived state,
Kassiber refreshes stale local journals first and includes the
`ui.journals.process` result in the tool result metadata. This refresh is local
and deterministic, and the same refresh path applies when the desktop GUI reads
journal-derived daemon kinds such as `ui.reports.*` or `ui.report.blockers`
directly. Watch-only source refresh remains explicit unless the active profile has
enabled automatic refresh-before-report maintenance, because refresh can contact
external services and import new transactions. Automatic refresh results are
redacted before they enter AI/UI tool metadata; exact backend URLs are not sent
to the provider. If any source refresh row fails, the maintenance/report-blocker
path returns a blocking `sync_failed` item instead of saying reports are ready.

Quoted or search-like questions can trigger `ui.transactions.search` before the
provider is called. Matching transaction notes, descriptions, counterparties,
tags, and values may then be included in the provider context; keep inference
local when those fields are sensitive.

Change-audit reads require an explicit previous-answer timestamp. Without a
baseline, `ui.audit.changes_since_last_answer` returns
`status: "baseline_required"` rather than claiming that nothing changed.
When a baseline is provided, the answer includes transaction metadata edit
events alongside transaction, wallet, journal, quarantine, and rate changes.
Bounded edit-history reads are available only through the safe
`ui.transactions.history` and `ui.activity.history` daemon tools; the assistant
does not get raw SQLite or CLI access.

The in-app prompt is a digest, not a full Agent Skill dump. It teaches the model the local-first accounting
role, the normal workflow order, the journal reprocessing rule, and the
boundary between read-only information and mutating actions. It also carries
one hard figure rule: when an overview's `fiat.completeness.costBasisComplete`
is false, the model must call cost basis and unrealized/realized gains
incomplete and name the blockers, never state them as exact (see
[the daemon overview contract](daemon.md)). The assistant is
skill-aware, but it is not shell-powered or CLI-powered: there is no raw command
execution, raw filesystem access, arbitrary daemon dispatch, or generic
Kassiber CLI tool.

Clients should upsert tool cards by `call_id`. Mutating tools emit an initial
`ai.chat.tool_call` with `needs_consent: true`, followed by
`ai.chat.tool_consent_required`. If the user approves the call, the daemon emits
another `ai.chat.tool_call` for the same `call_id` with `needs_consent: false`
before `ai.chat.tool_result`; that second record marks the approved call as
running and must not create a duplicate card.

Live chats advertise capability packs selected from the latest question and
typed `screen_context`. `tool_profile=core` intersects those packs with the
common catalog; `scoped` includes relevant specialist tools; `full` skips
capability scoping. Omitted `tool_profile` defaults to `scoped` at the daemon;
the CLI supplies `core` by default. Profile selection limits exposure and
context size, not the validation or consent required to execute a tool.

Small local models are the reason the catalog stays bounded: every schema
together is about 36k tokens, more than many local context windows. Instead,
the Kassiber system prompt ends with a static area index (`tool_area_index`,
about 300 tokens), one line per area, so the model knows what exists without
the schemas. The index is identical on every turn, which keeps provider prompt
caches valid. A capability question such as "what can you do?" is answered from
that index and no longer advertises the full catalog.

When routing missed an area, an HTTP provider's model can call `load_tools`
with up to four area names. The daemon adds those schemas, under the same
`tool_profile` limits and on-device restrictions, to both the advertised set
(which authorizes calls and feeds the privacy receipt) and the next round's
tool list. CLI agents behind the broker fix their catalog when a turn starts
and abort on unknown tools, so they never get `load_tools`; their next
question is routed again. `full` has nothing to load. Per-area schema budgets
in `tests/test_ai_tool_areas.py` keep areas from growing unnoticed.

Tax figures come from the core, never from the model's recall of tax law:

- `ui.reports.capital_gains` takes `tax_year`, `asset`, and `limit` (default
  50, at most 200). It returns the year's totals, the Austrian Kennzahl rows,
  and the newest disposal and income rows. Each row carries its transaction id,
  asset, exact msat, Austrian category, and Kennzahl. The rows are journal
  entries, not acquisition lots: Austrian Neubestand uses one moving-average
  pool, and the payload says so.
- For an Austrian EUR book, `ui.reports.tax_summary` adds `indicative_tax` for
  the requested year, or the latest year. It is 27.5% of KZ 172 + KZ 174 −
  KZ 176 as reported, from `kassiber/core/tax_estimate.py`, and is zero when
  that base is negative. Any report blocker withholds the figure
  (`status: "withheld"`, `estimated_tax_eur_cents: null`). KZ 801 Altbestand
  speculation is listed as an amount and never estimated, because it is taxed
  at the progressive rate. The notes say the figure covers this book only and
  does not deduct KESt an Austrian provider withheld. The desktop's own tax
  summary read skips this check.

The authoritative tool names, schemas, effects, and daemon mappings live in
[TOOL_CATALOG and get_tool](../../kassiber/ai/tools.py), with capability selection
in `select_tool_capabilities` in the same module. These are distinct from
CLI `commands describe` and the renderer allowlist. Do not reconstruct a tool
schema or assume that a daemon kind is available to the model.

Desktop chat builds an ephemeral `screen_context` from a positive registry of
canonical routes and capability packs. It contains only a route and,
when available, a typed entity id, bounded filters, explicit capability
hints, or page `metrics`. Sensitive keys and oversized filters are rejected.
`metrics` are bare counts the page is showing, keyed by the allowlist
`SCREEN_CONTEXT_METRIC_DEFINITIONS` in
[ui_snapshot](../../kassiber/core/ui_snapshot.py); the daemon rejects other
names and non-count values. Metrics travel with the `metrics_scope` the page's
snapshot was computed on (workspace, profile, journal input version, whether
reviewed pairs were counted as one item, and the id of the one wallet the
page's wallet filter selects; never a descriptor, xpub or wallet
configuration). The metric definitions the model reads say how pairs were
counted, and counts made under another journal state are dropped. The Transactions list, its dashboard, the
review-queue counts and this binding resolve the wallet filter through one
function, `resolve_ui_wallet_filter`: a filter matching several wallets, by id
or label in any combination, is ambiguous, so the page counts all of them and
sends no metrics scope. When the chat starts, the daemon drops the metrics
unless that scope still matches the active book and its current input version
and the wallet id still resolves to that wallet alone. The model sees the
wallet as the id and label `ui.wallets.list` already exposes (`null` for every
wallet), never the binding ids; the whole context, label included, goes through
the same secret, path and URL redaction as AI tool results. The model gets each metric's definition and is
told the counts are what the screen showed, not a tool result: a mismatch with
a tool read may come from scope (period, wallet), definition, or a screen that
is older than the read, and the model names a cause only when the tool results
show it. The desktop publishes no metrics while the Transactions dashboard
still shows placeholder data kept from a previous query, such as the previous
book's snapshot right after a book switch. The Transactions
Review queue card, its `quick=review_queue` filter, and `ui.workspace.health`
`review_queue` share one status definition there and count the same items
through `transaction_review_queue_counts`: while journals are current, the two
legs of a reviewed transfer or swap are one item, which takes its most urgent
leg's status; with stale journals each leg counts separately, and health's
`review_queue` says which (`pairs_grouped` and its `scope` text). A status the
user set wins over the chain, so "pending" means marked pending or, with no
status set, no confirmation timestamp; it is not a quarantine. The rule is
`transaction_status_sql` in the core, which the CLI's status filters use too,
and each Transactions row carries its result as `queueStatus`: for a reviewed
pair, the most urgent status of its legs inside the page's wallet and date
filter, the same population the counts use (`_review_queue_legs_sql`). The
list's status filter and its Review queue and Failed import filters select on
that status, and the transaction detail, the overview's rows and its chart
events show it. The CLI's `transactions list` lists legs, so its status filter
stays per leg. The desktop
shows that value but never saves it: the edit form changes the authored
`reviewStatus`, and a save sends `review_status` only when the user changed
it.
The context is
inserted as untrusted navigation state immediately before the current user
turn; it never grants filesystem access and is not a replacement for a typed
read tool. An "Ask about this" action pins a typed entity for the following
turns until the user removes it; the item's identity travels only as that
typed entity, never interpolated into the prompt text. A question queued
while an answer streams, or held until the open book's provider settings have
seeded the reasoning effort, keeps the pin that was active when it was
submitted and the page it was asked on (route, capability hints, and counts
with the book state they were counted on, which the daemon still checks);
navigating, or pinning or clearing another item, before it runs does not
retarget it.

Everything the assistant sends belongs to a book, and one rule binds it
(`BookStamp` in `ui-tauri/src/components/ai/bookStamp.ts`). A stamp is the
daemon session plus the book's workspace and profile ids. It is captured
synchronously at the user's action that creates the content, before any
await: the submit for a typed question or held edit, the click for a pin, a
page's handoff or a reopened chat, the click before the picker for an
attached file. A page's handoff takes the book of the page's scope boundary
(the book its data came from) when it has one, otherwise the open book. The
stamp travels with the content, and a request is sent with its content's
stamp, never the one current when it is sent. When the open book's ids are
not known yet, the action is refused with a notice, and nothing is stamped
later. Page handoffs start with `beginAssistantHandoff` before they await
anything (a graph projection, a preview); a handoff cannot be sent without a
stamp. Creating a book opens it without a new daemon session, so the ids are
compared too, not only the session. Every send checks the stamp
synchronously, so nothing from a book the user left (by a switch or by
creating a book) is sent, even when the change lands in the same render as
the send, and the daemon refuses a request whose `expected_scope` names a
book that is no longer open. Nothing made in one book is
sent in another: a queued question or edit is not sent, its pin is dropped
and its text goes back to the composer (once: a prompt already there as a
whole entry, not merely inside the user's own text, is not added again); a
queued
or parked handoff is dropped with its text, because a page wrote that text
from the old book's data (transactions, recipes, grants, snapshot ids). A
parked handoff is cleared from the composer on the switch, and the send path
rejects one that races it. A notice says what happened. A page's own
request to the assistant (the quarantine fix, Source of Funds, chain analysis,
PSBT and dataset handoffs) runs with the screen context of the page that made
it, captured when it is made, and clears the pin, so a pin never replaces the
tool pack that request needs. Made before a model is chosen, it waits in the
composer with that context, which survives the user's own edits of its text
and is dropped when the draft is cleared or any text is put in its place (Ask
about this, a starter question or suggestion on any composer, a recalled or
rewound prompt) or the book changes. An evidence continuation likewise uses its own
request's context for that turn but leaves the pin for the user's next
question, since it continues the same conversation. As a second check, when the chat starts the daemon drops a `transaction` entity
whose id or txid is not a transaction of the active book, so the model is never
told about an item from another book.

Only schemas advertised for that turn may be requested by the provider. The
small deterministic pre-read router remains separately bounded and read-only.
The daemon validates tool
arguments against the catalog again at execution time, including required
fields, types, enums, numeric and exclusive bounds, string lengths, patterns,
item counts, and `additionalProperties`; provider output cannot
smuggle a hidden network or mutation argument into a narrower tool.

Read tools return bounded, redacted projections. The following boundaries
matter across tool additions:

- UTXO/privacy projections omit addresses, scripts, derivation details, wallet
  material and backend endpoints. Privacy Mirror and chain analysis share the
  same snapshot and opaque provider references; use `ui.chain_analysis.ai_context`
  for handoffs. Dataset import requires a validated source/recipe/hash-bound
  preview, full agent access and once-only consent. See
  [chain analysis](local-chain-analysis.md) and [Privacy Mirror](privacy-mirror.md).
- Cross-book context requires an explicit book-set request within the chat's
  original workspace. Keep each book's tax/lot scope and mixed fiat values
  separate. Ordinary health remains active-book scoped.
- Report tools return processed journal facts. Rates remain local unless live
  access is enabled; a rate rebuild can complete while journal processing fails,
  and must report those outcomes separately. [Tax and journals](tax.md) owns
  report semantics and readiness.
- Transaction review combines independently degradable evidence sections.
  Attachment lists omit paths and URL targets, merchant reads omit raw provider
  payloads/document bytes, and egress summaries omit destination identities and
  request contents. [Source-of-funds review](source-of-funds-review.md) owns the
  provenance workflow; evidence never acquires custody or tax authority.
- Transfer suggestions do not write decisions. Ownership intent remains
  necessary for Bitcoin rail changes; an external payment is not a self-transfer.
  `ownership_graph` candidates require individual review and cannot enter bulk
  or rule pairing. A bounded review packet may omit competing rows: use
  `conflict_set_id` and the full cluster before approving a choice. Candidate
  evidence retains contradictions and exact declared amounts so a `strong`
  result is explainable. [Swap matching](../../kassiber/ai/skill_references/swap-matching.md)
  owns the review procedure and filter semantics.
- Loan open-lock hints remain heuristics, never liquidation proof or automatic
  marks. Bounded lists must disclose full counts and truncation.

`ui.workspace.health` summarizes the active books set and book
(`workspace`/`profile` internally), wallet and transaction counts,
journal freshness, quarantine count,
and report-readiness hints from the current database. `ui.next_actions` returns structured
recommendations such as create a wallet, sync/import, process journals, review
quarantine, or run reports. It only advises; it does not execute those actions.

Counters carry their meaning. `ui.workspace.health`, `ui.review.badges`,
`ui.journals.quarantine` (in `summary`) and `ui.journals.quarantine.groups`
include `definitions`, one sentence per counter, so the model can say why the
review badge, the review queue and the quarantine total differ instead of
guessing.

Journal-derived figures that are stale reach the model as an explicit state,
never as zeros. After the read's own journal refresh (or on an MCP read, which
never refreshes), `ui.journals.transfers.list`, `ui.reports.capital_gains`,
`ui.journals.events.list` and `ui.journals.snapshot` replace their figures
with `null` plus `status: "unavailable"`, a `reason` (`journals_stale` or
`journals_never_processed`) and a `remediation` naming `ui.journals.process`;
the overview nulls its basis, unrealized and realized figures when its
completeness is `stale`. Quarantine reads keep the last processing's rows
with `projection.status: "stale"`, or are unavailable if journals never ran.
The desktop pages show their own stale state and are unchanged; this lives in
`_project_stale_journal_figures_for_ai`.

Quarantine can be read and acted on by root cause.
`ui.journals.quarantine.groups` returns whole-book counts and per-asset msat
totals by reason and by root group (a root transaction plus every downstream
row, such as `custody_basis_barrier`, that waits on it), largest first; with a
`group_id` (`root:`, `unrooted:` or `reason:`) it pages that group's members
in a stable order behind a cursor bound to the journal state, which fails with
`quarantine_cursor_stale` once the quarantine changed.
`ui.journals.quarantine.resolve` accepts a `root:` `group_id` instead of a
transaction and acts on that root. After its reprocess it reports
`dependents_cleared` (count and up to 50 ids of rows that waited on the root
and are no longer held) and a `quarantine_delta` that also lists rows that
cleared outside the group or newly appeared; without a reprocess the effect is
`not_measured`, not zero. The CLI has the same reads and effect
(`journals quarantine groups`, `journals quarantine resolve ... --group-id
--reprocess`).

`ui.transactions.verify` (CLI `transactions verify`) checks one transaction
against the raw graph stored with it, locally and without addresses: inputs
minus outputs against the recorded fee, and every wallet of the book owning an
input or output against its imported row's wallet delta, and the reverse. It
returns `ok`, `mismatch`, `partial` or `unverifiable` (no stored graph) with
expected and recorded msat, and per input/output `owned`, the wallet and a
`keychain` of `receive`, `change` or `unknown` taken from the owning wallet's
derivation branch. Ownership comes from the book's local index, so an
unmatched output is "not matched", never "external".

`read_skill_reference` is a virtual tool. `read_skill_reference("index")`
returns a compact routing document derived from the Kassiber skill concepts and
points the model to deeper allowlisted references. The deeper references are
restricted to packaged files under `kassiber/ai/skill_references/`:
`command-templates`,
`journal-processing`, `metadata`, `onboarding`, `reports`,
`secrets-and-backup`, `swap-matching`, `troubleshooting`, `verification`, and
`wallets-backends`.

Tools marked mutating or egressing require consent. Their catalog metadata,
not a copied list of names, determines the execution path. Reviewed same-asset
pairing can express one-to-many or many-to-one ownership hops; cross-asset and
layer-transition links remain one-to-one on that pairing surface.

Natural-language pairing ("pair the Phoenix payment with the Liquid receipt") is
supported on the `core` tool profile through `ui_transfers_pair` only.
`ui_transfers_bulk_pair` is excluded so a single consent can never sweep a whole
review queue, and `ui_transfers_unpair` is excluded too — the assistant is told
not to promise an undo it may not be able to perform, since the user can always
unpair from the desktop review screen.

`ui_transfers_dismiss` is on the `core` profile too, behind per-call consent.
A dismissal still has no undo, but it can be read back:
`ui_transfers_dismissals_list` (`transfers dismissals list`) returns each
dismissed pair with both legs' full ids, wallet, direction, amount and
`expires_at` (null means never). `ui_transfers_suggest` and
`ui_transfers_review_context` return profile-wide `exclusions`, counted before
filters: `excluded_by_active_pair` and `excluded_already_journaled` count legs,
`excluded_by_dismissal` counts pairs. Zero candidates is therefore never
silent.
A full-access provider can also propose a bounded dismissal (1 to 3650 days)
inside a review batch, where the consent preview shows it and the receipt
records it.

Three properties of `ui_transfers_pair` are documented on the tool because the
model would otherwise have to guess them, and each guess is a real error class:

- `policy` decides tax treatment (`carrying-value` carries basis across the move
  and realizes nothing; `taxable` realizes a disposal). Both values now carry an
  explanation and the model is told to take the candidate's `default_policy` and
  state which one it used rather than inferring.
- `out_amount` is a decimal **BTC** string — the only non-msat amount in the AI
  surface, next to `out_amount_msat` fields the model has just read. The
  parameter doc carries an explicit unit warning.
- unlike `ui_transfers_bulk_pair`, which refuses any candidate with
  `conflict_size > 1`, this tool *will* pair one member of a conflict cluster and
  thereby consume both legs, making the competing candidates unauthorable. The
  description requires the model to check `conflict_size`, list the competing
  candidates, and let the user choose. Blocking it outright would break the
  desktop flow, where the human resolves the cluster by choosing; disclosure at
  the consent surface is the remaining gap.

The custody-component resolver handles 1:N, N:1, N:M, multi-hop, and
missing-wallet histories atomically. `ui_transfers_components_plan` and
`_apply` take the same `action` as the CLI: `create`, `revise` (a new revision
of a lineage, which may reuse the old anchors and with `activate: true`
supersedes the active revision atomically), `supersede`, `undo`, and
`activate`. Apply re-plans against the current book and fails closed when the
input version moved. A revision spec from the model is validated against the
create contract, and a conversion cannot activate or attest its own review.
These are authored component mutations, not generic AI pairing shortcuts.

`ui_review_plan` is the what-if path and the batch path. It applies up to 50
typed operations to an in-memory copy of the book and never persists, so it
needs no consent; `ui_review_apply` applies the unchanged artifact atomically on
one consent. The artifact's `before`/`after` carry per-year realized gain/loss,
income and, for Austrian EUR books, Kennzahlen and the core's indicative tax
(withheld while report blockers remain), and `delta` lists the years that
change. The model answers "what does this change do to my taxes?" from that,
not from its own arithmetic. With full agent access a batch may also create
pairs, dismiss candidates, revise/undo/activate/supersede components, run
custody-gap actions and declare acquisition kinds; a restricted provider keeps
prices, exclusions, unpairs and component creation. Conversion components
stay drafts for the owner in either mode. Operation shapes and the tax fields
are in [custody resolution](custody-resolution.md#resolve-with-the-cli-or-chat).

A chat attachment can be imported, not only analyzed.
`ui_wallets_import_attachment_plan` runs the desktop's generic-ledger import of
the staged file inside a rolled-back savepoint and returns its exact effect
with a plan digest, the file's SHA-256 and the book's input version;
`ui_wallets_import_attachment_apply` repeats it and commits only when all three
still match, after once-only consent. The consent card shows the plan the daemon
recomputes then, and its summary names the row count and target. Only file
and exchange wallets, or a new `custom` wallet, are targets. Applied imports
are ordinary import runs: `ui_imports_list`, `ui_imports_rollback_plan` and the
consented, plan-bound `ui_imports_rollback` undo them, except when a rollback
would delete reviewed work, which stays in the desktop. No tool takes a path,
and results never carry one. A restricted provider gets counts and the column
plan, without dates, amounts or the filename. Descriptor, xpub, address and
credential-backed connections are never created by the assistant:
`ui_wallets_request_input` returns a typed handoff the desktop shows as a setup
button. `ui_wallets_list` gives each wallet a `completeness_class` and
`scan_bounds`, so the assistant can say where a wallet's history is unknown
rather than assume it is complete. Details:
[imports](imports.md#onboarding-an-unsupported-exchange-with-the-assistant) and
[wallet completeness](imports.md#wallet-completeness-and-scan-bounds).

Source-funds evidence writes are also consent-gated:
`ui_source_funds_sources_create`,
`ui_source_funds_links_create`, `ui_source_funds_links_review`,
`ui_source_funds_suggest`, `ui_source_funds_links_bulk_review`,
`ui_source_funds_sources_attach`, and `ui_source_funds_links_attach`. Evidence
attach tools accept only existing managed attachment ids, never paths. These tools
create/review provenance evidence only; they do not mutate tax/journal
`transaction_pairs` and they support non-CoinJoin link types such as
self-transfer, exchange transfer, trade, swap, peg-in/peg-out, Lightning hops,
manual source, and missing-history edges. CoinJoin/PayJoin links should stay
explicit about privacy-hop ambiguity unless the user has reviewed stronger
evidence. `ui_source_funds_assemble`, `ui_source_funds_cases_save`, and the
virtual `ui_source_funds_export` complete that workflow; export results sent to
the model include the filename/format but not the managed local path.

Transaction review writes (`ui_transactions_metadata_update`,
`ui_transactions_history_revert`, `ui_attachments_copy`) preserve the
append-only edit/evidence audit trail and run only after consent. Commercial
review writes (`ui_btcpay_provenance_review`, `ui_documents_create`) are also
consent-gated. The virtual `ui_reports_export` maps a small report/format enum
onto existing deterministic PDF/XLSX/CSV/audit-package exporters and likewise
withholds managed paths from model context. Stale journals may also be
refreshed automatically before read/report tools as local maintenance. Wallet
sync before report reads is disabled by default; it runs automatically only
after `ui_maintenance_configure` enables that active-profile setting, or when
the user explicitly approves a maintenance/sync call. Tool-call arguments are
redacted before previews, stream events, auto-context entries, and tool-result
content are returned to the model/UI. When a model requests a mutating tool, the
daemon emits
`ai.chat.tool_consent_required` with a short summary and redacted argument
preview, then waits for:

Loan review writes (`ui_loans_mark`, `ui_loans_link`, `ui_loans_unmark`) are
also consent-gated and invalidate journals. Open-lock heuristics never create
marks automatically.

Suggestion seeding (`ui_btcpay_provenance_suggest`, `ui_source_funds_suggest`)
is consent-gated too: both upsert unreviewed link rows and commit, so neither
counts as a read. `ui_review_worklist` stays read-only by listing pending
suggestions through `ui.btcpay.provenance.links` instead of seeding new ones.

The daemon freezes the project/database and active workspace/profile ids when
the chat starts. Every read and approved mutation rechecks that scope on the
main SQLite thread immediately before execution. If the user switches projects
or books mid-turn, the operation fails with `stale_context`; history persistence
still targets the original book and never the newly active one.

A turn can also be refused before it starts. Every desktop `ai.chat` request
carries `expected_scope`, the workspace and profile ids of the book the
question was asked in: its content's stamp, captured at the user's action
that made it (see the book rule above), and the evidence request's own book
for an evidence continuation. A turn whose book is not known is not sent. The
desktop holds sends until it knows the open book's ids
(`ui.profiles.snapshot`). If another book is open when the request arrives,
for instance because a book switch reached the daemon first, the daemon
refuses it with `stale_context` before the provider is resolved or any tool
runs. The desktop handles that like its own book check: the turn leaves the
transcript, the user's own text returns to the composer, a handoff's text
(written from the old book's data) is dropped, and a notice says which. The
CLI's `kassiber chat` runs in one book and sends no scope.

A desktop conversation belongs to the book it started in. The daemon cannot
check an Incognito or history-off transcript, which the desktop sends whole
with every turn, so the desktop does not carry it over. Opening another book
(a new daemon session, or other book ids before the session counter moves)
starts a fresh conversation. A conversation is bound to a book by its first
turn, its first attached file, or a reopened chat. The transcript, the saved
chat it continued, a staged attachment and pending evidence requests stay
with the old book; a notice says when an attached file was removed. A file is
staged for the book it was chosen in (the picker sends that `expected_scope`),
and one picked while another book opened is not attached. A
turn still running there is cancelled, and its late result is ignored. The
Incognito choice and the user's own draft text stay; a parked handoff does
not. The history list then shows the new book's chats. As a backstop, the
send path refuses a turn whose transcript, saved chat or attachment belongs
to a different daemon session or book ids than the turn names. The user's
text returns to the composer, as for a queued question. Reopening a saved
chat sends the open book as `expected_scope` with `ui.chat.sessions.get`, and
a reopened chat belongs to that book.

```json
{
  "kind": "ai.tool_call.consent",
  "args": {
    "target_request_id": "<active ai.chat request_id>",
    "call_id": "<tool call id>",
    "decision": "allow_once"
  }
}
```

`decision` can be `allow_once`, `allow_session`, or `deny`. Session consent is
in-memory and lasts only for the current `ai.chat` request; it applies only to
subsequent calls to the same tool name in that chat. If the user denies or does
not respond before the consent timeout, the daemon feeds a tool result back to
the model with `ok: false` and `reason: "user_denied"` or
`"consent_timeout"`. Unknown tools still return `tool_not_allowed` and are not
executed.

A failed tool returns `ok: false`, `reason` (the error code), `message`, the
error's `hint`, and a `remediation` when Kassiber knows the fix:
`{action, target, tool, arguments_hint, desktop_only}`, with full resolvable
ids in `target` and the function name in `tool` (null when no tool can take
the step; `desktop_only` when only the user can, as for restricted agent
access). Core errors attach it to `details["remediation"]`, which the CLI
envelope also shows; the daemon adds defaults for stale plans, changed review
cases, restricted access, and a switched book. Error `details` reach the model
only for an allowlist of id-level codes (such as `ambiguous_reference` and
the custody component codes), bounded and redacted like any result.

A transaction reference is a full Kassiber id or an external id (txid) that
names one row. When several rows share a txid, as in a CoinJoin or a batched
deposit, `ambiguous_reference` lists every candidate; the qualified forms
`<txid>:<wallet id or label>`, `<txid>:in|out` and `<txid>:<wallet>:<in|out>`
select one and still fail closed when they do not. Prefixes never resolve.

The terminal `ai.chat` record includes a compact `provenance` object with the
provider/model, generation timestamp, local tool names used, journal refresh
status, sync-attempt status, successful versus denied tool attempts, and counts
learned from health/report-blocker tools. Denied calls never count as executed
or as cross-book disclosure. The GUI uses that object and the exact tool payloads to render source
chips beside the assistant answer, making the result traceable to
program-derived facts.

### Visuals in chat

The assistant can answer with an interactive visual through the read-only tool
`ui_visuals_render` (`ui.visuals.render`). It is an ordinary catalog tool, so it
reaches every provider the same way: HTTP function calls, Codex `dynamicTools`,
and the MCP bridge for Claude, OpenCode and Copilot. It belongs to the `reports`
and `transactions` capability packs and to the `core` profile. A question that
names a visual ("chart", "plot", "graph", "Diagramm", "Grafik", …) selects the
reports pack, and a quarantine review keeps its own bounded pack without it.
The model chooses only what to show: a `source`, a `measure`, the source's
filters and an optional `title` of at most 80 characters (control characters
removed). It never supplies numbers, markup or code. The daemon reads every
figure from the same local reads the app's pages and report tools use, and
[`kassiber/core/chat_visuals.py`](../../kassiber/core/chat_visuals.py) turns them
into a typed specification, long-format `rows` and source-specific extras:

| `source` | Read | Visual | Filters |
| --- | --- | --- | --- |
| `overview` | `ui.overview.snapshot` | portfolio value, BTC balance, cost basis, unrealized and realized-this-year results, value vs cost basis over time, allocation by wallet, completeness | none |
| `balance_history` | `ui.reports.balance_history` | balance per asset over time; amount, value, cost basis, or value vs cost basis | `interval`, `start`, `end`, `wallet`, `asset`, `limit` (buckets, default 60, at most 240) |
| `portfolio` | `ui.reports.portfolio_summary` | holdings by wallet or asset; value, amount, cost basis or unrealized | `group_by`, `asset` |
| `tax_summary` | `ui.reports.tax_summary` | realized result per tax year, with each year's breakdown by asset and holding period | none |
| `cash_flow` | `ui.transactions.dashboard`, with the page's transfer suggestions | received and sent per period, in BTC or fiat; transfers and swaps counted apart | `period`, `wallet` |
| `transactions` | `ui.transactions.list` | a list of up to 25 transactions that open in the app | `query`, `direction`, `wallet`, `period`, `status`, `sort` (recent, oldest, largest, smallest), `limit` |
| `transaction_flow` | `ui.transactions.graph`, local only, and `ui.transactions.resolve` | the transaction detail panel's own graph, with the change to the user's wallets, the amount moved and the fee above it, or the transaction itself when there is no graph | `transaction` (required) |
| `readiness` | `ui.report.blockers` | a checklist of what blocks reports, each open item with an action | none |

A filter that belongs to another source is refused, not ignored. Every row
carries all of its source's measures, so the user switches measure, grouping,
range or unit in the visual without another read or another model turn. Before
a report source (`balance_history`, `portfolio`, `tax_summary`, `overview`) is
read, stale journals are refreshed exactly as for the report tools, which
includes the wallet sync a user may have enabled for report reads. The
transaction list, flow, cash-flow and readiness visuals run no such
maintenance: they are plain local reads.

Visuals state what they know is missing as `caveats` instead of plotting it as
zero. Report visuals read the same blockers as the readiness checklist and
flag stale journals, quarantined transactions, unresolved custody, transactions
without a price (so basis and gains may be incomplete) and a failed sync.
Within a visual: a held balance with no price (the value is `null`, never 0),
no current market rate
(the overview then shows no value), unpriced movements counted as zero fiat on
the cash-flow page, confidential amounts, and data cut off by a limit. The
overview marks cost basis and gains "Incomplete" whenever its completeness
block says so. The balance report leaves out every bucket in which a balance is
zero, so the visual builds its buckets from the calendar range and puts each
zero back: a full exit falls to zero instead of ending on the last balance held.
When the report truncated by row, the visual drops the oldest bucket, which may
have lost some assets. A transaction flow names only the user's own wallets
(from the graph's wallet field, also for a coin several of them share); other
inputs and outputs are "Input n" and "Output n", with no address or outpoint,
and the graph is read without public lookup. Amounts of different assets are
never added up as bitcoin. Cash-flow buckets are counted in UTC; the
Transactions page counts them in the desktop's time zone, so a movement close
to midnight can fall in a neighbouring bucket.

The model reads a digest instead of the full payload (`model_digest`): the
specification, the caveats, summary figures such as each series' first, last,
lowest and highest value, and the rows only while there are at most 30. The
digest says the visual was shown to the user, and its `summary` repeats the
caveats, because the auto-read context keeps only that part of an oversized
entry. The desktop and `kassiber chat` receive every row. CLI agents get the
same digest as HTTP providers.

The pre-read router plans a visual the user asks for by name, so a model that
never calls tools still answers with one: a chart word plans the chart its
subject names (tax, holdings, cash flow, otherwise the balance); "overview" or
"dashboard" plans the overview; "what needs attention", "checklist" or "ready
to file" plans readiness; and "show my transactions" plans the list, with
"largest", "oldest", "incoming" and "outgoing" taken as its sort and direction.
"graph" next to ownership, evidence or chain-analysis words is not a chart. A
visual the router drew and the model then drew again with the same data is
shown once, as the model's.

The desktop renders each visual as a card under the work log, above the answer
(`ui-tauri/src/components/ai/visuals/`), with a placeholder while the daemon
draws it. Cards animate in: figures count up and lines and bars draw from their
baseline; the global reduced-motion rule stops all of it. A transaction flow
is drawn with the transaction detail panel's flat bowtie
(`TransactionFlowDiagram`), never the panel's draggable 3D view. The desktop
reads that graph itself with the panel's own local read (`ui.transactions.graph`
without public lookup), so the chat and the panel draw the same data while the model only
ever receives the redacted summary without outpoints or addresses. Every card has a table view, "Copy as CSV"
(raw numbers; a text cell that starts with a formula character gets a leading apostrophe),
"Open larger", and "Open in Kassiber" for the matching page or transaction.
Follow-up chips and each readiness action send an ordinary prompt through the
assistant's send path, so a write it leads to still needs consent. The desktop
accepts only a version 1 payload with known sources, units, checks and actions
and shows the tool card for anything else. Series are keyed `s0`, `s1`, …, so no
data label becomes a CSS or object key, and the model's title is plain text.
Colours were run through the dataviz palette validator for both themes; gains
and losses, and received and sent, use a blue/red pair that stays apart under
colour-vision deficiency, and every bar chart's axis includes zero. Hidden
amounts blur every value and axis label. Chat history keeps an answer's text,
not its tool results, so a reopened chat shows the answer without its visuals;
asking again draws them from the current book. `kassiber mcp serve` does not
offer the tool, since external agents have no surface to draw on.

## Remote inference

Remote inference requires an explicit privacy decision. Provider branding or a
local proxy alone does not establish confidentiality. Verify the selected
transport and provider protections against the
[privacy model](privacy-and-security.md#ai-provider-configuration).

## Related files

- [Kassiber CLI skill](../../skills/kassiber/SKILL.md)
- [`../../kassiber/ai/client.py`](../../kassiber/ai/client.py)
- [`../../kassiber/ai/broker_client.py`](../../kassiber/ai/broker_client.py)
- [`../../ui-tauri/provider-broker/`](../../ui-tauri/provider-broker/)
- [`../../kassiber/ai/contracts.py`](../../kassiber/ai/contracts.py)
- [`../plan/08-external-document-reconciliation.md`](../plan/08-external-document-reconciliation.md)
- [Privacy & security](privacy-and-security.md)
