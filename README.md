# ingfah

A command-line client for the [Ingfah](https://ingfah.ai) client API, authenticated with an API key. It is for Ingfah client developers and for AI coding agents that drive the API from a shell. Every command prints JSON when its output is not a terminal, and every error names what to do next.

The workspace has two crates:

- `crates/ingfah-client` is the library: route registry, transport, errors, and the agent-revision logic. It has no CLI or printing code, so another front end (an MCP server, for example) can reuse it.
- `crates/ingfah` is the `ingfah` binary.

`spec/openapi.public.v1.yaml` is the vendored integration API spec. A test checks the route registry against it.

## Install

```sh
cargo install --path crates/ingfah
```

Shell completions:

```sh
ingfah completion zsh > ~/.zfunc/_ingfah   # also bash, fish, elvish, powershell
```

## Set up a key

Create a key in the dashboard under การตั้งค่า → API Keys. The dashboard shows a key once. A key acts as the admin who created it, with the scopes it was given.

```sh
ingfah auth login            # hidden prompt
ingfah auth status           # key source, masked key, base URL, and a test call
ingfah auth logout           # delete the stored key
```

`auth login` checks the key format (`sk-ing-v1-` and 64 hex characters, or a legacy `ing_sk_` key). It then makes one test read (`GET /client/ai-agents?per_page=1`) and stores the key in the system keyring only if the read succeeds. To read the key from a pipe instead, pass `--with-key-stdin`.

The CLI looks for a key in this order:

1. `--api-key`
2. `INGFAH_API_KEY`
3. The keyring entry for the profile (service `ingfah-cli`, account = profile name)

If none of these has a key, the command fails before it sends anything.

### Profiles

One key belongs to one client, so use a profile per client:

```sh
ingfah --profile acme auth login
ingfah --profile acme agents list
INGFAH_PROFILE=acme ingfah calls list --from 2026-10-01T00:00:00Z
```

A profile can also pin a base URL. Run `auth login --base-url URL` to save it in `~/.config/ingfah/config.toml` (or under `$XDG_CONFIG_HOME`):

```toml
[profiles.acme]
base_url = "https://api.ingfah.ai"
```

The CLI picks the base URL in this order: `--base-url`, `INGFAH_API_BASE_URL`, the profile's `base_url`, then `https://api.ingfah.ai`. The URL must use https; plain http is allowed only for `localhost`, `127.0.0.1` and `::1`.

## Commands

| Command | What it does |
|---|---|
| `teams list`, `teams get ID` | AI agent teams (products) |
| `agents list`, `agents get SLUG` | Agents, with the published prompt and the bound tools |
| `agents create --visibility private\|public ...` | Create an unpublished agent with a draft revision |
| `agents revisions list SLUG`, `agents revisions get SLUG ID` | Revision history and content |
| `agents revise SLUG ...` | Create a draft revision that changes only the fields you name |
| `agents publish SLUG --revision ID` | Put a draft live, then check its tools and voice |
| `tools list`, `tools get ID` | Plugin functions |
| `phone-tools list`, `text-chat-tools list`, `voices list` | Things an agent can link to |
| `calls list`, `calls get UUID`, `calls recording UUID` | Calls, transcripts, recordings |
| `templates list`, `templates get ID` | Customer templates (outbound options) |
| `batches get`, `records`, `create`, `pause`, `resume`, `cancel` | Outbound batches |
| `analytics summary --from --to` | Call totals |
| `api METHOD PATH` | Any other allowed route |

List commands take `--page`, `--per-page` and `--all`. With `--all`, the CLI fetches pages until `has_next` is false, the last page is reached, or a page comes back empty.

`api` reaches every route in the registry, including routes that the v1 document leaves out but that accept an API key: text-chat inboxes, text analytics, channel configs and automations. Pass the query with `-q key=value` and the body with `--body` (inline JSON, `@file`, or `-` for stdin):

```sh
ingfah api GET /client/text-chats/conversations -q 'status[]=open' -q per_page=50
ingfah api GET /client/chat-sessions-list/csv -q start_time=2026-10-01T00:00:00Z --out calls.csv
```

## Safety model

Every request resolves to a row in the route registry (`crates/ingfah-client/src/routes.rs`). Each row records the route's scope and its effect:

| Effect | Meaning | Examples |
|---|---|---|
| read | Reads only | All GETs |
| write | Changes configuration that customers see only after a publish | Create a revision, update a team |
| live | Changes what customers get right away, or dials people | Publish, create, pause or resume a batch, test-run a tool, update a tool, toggle a bot |
| destructive | Deletes or ends something for good | All DELETEs, cancel a batch |

The CLI applies these rules to every request:

- The CLI refuses a method and path that are not in the registry before it sends anything. This covers JWT-only routes, such as the knowledge base, and `/get-call-token`.
- On a terminal, write, live and destructive calls show the method, the path, the effect, what the call does, and the body, then ask y/N. `--yes` skips the prompt. Without a terminal, the CLI refuses the call unless you pass `--yes`.
- `--dry-run` prints the method, path, effect and JSON body of a write, sends nothing, and exits 0. Reads still run, so `agents revise --dry-run` can show the real body it would send.
- `--out FILE` never overwrites a file. The CLI does not print binary bodies to a terminal.
- The CLI never prints the key or a presigned recording URL at the default verbosity. `calls recording` shows the URL only with `--show-url`. `-vv` traces show the key masked.
- Only GETs are retried: three attempts on 408, 429 and 5xx, with backoff and `Retry-After` honoured. The CLI never retries a POST, PUT, PATCH or DELETE.

## Create an agent

`agents create` builds the body from the same `--set`, `--set-file`, `--from-file` and `--add-tool` flags as `agents revise`:

```sh
ingfah agents create --visibility private \
  --set name=Mali --set vocal_name=มะลิ --set ai_greeting_message=สวัสดีค่ะ \
  --set-file ai_instruction_identity=identity.md --set-file ai_instruction_task=task.md \
  --set voice_id=3 --add-tool plugin:12 --add-tool phone:5
ingfah agents create --type text --visibility public --from-file agent.json --add-tool text:1
```

`--type` is `audio` (the default) or `text`. `--visibility` is required. The required fields are `name`, `ai_greeting_message`, `ai_instruction_identity` and `ai_instruction_task`, plus `vocal_name` for an audio agent. The optional fields are `profile_url`, `description`, `prompt_engine_version`, `ai_instruction_flow`, `ai_instruction_variables`, `ai_instruction_default_variables`, and, for audio agents only, `voice_id` and `voice_speed`. `ai_instructions` cannot be set on create; set it later with `agents revise`.

Before it sends anything, `agents create` refuses (exit 3):

- a body without one or more required fields, naming every missing field;
- a tool kind that does not apply to the type (`phone` on a text agent, `text` on an audio agent);
- a flow node that names a tool the body does not link.

After the create, it reads the agent back and compares its draft revision (`ai_agent_instruction_revision`) with what it sent. A mismatch exits 1. On success it prints the slug, the agent id, the draft revision id, and the publish command. The new agent's top-level prompt fields stay empty until you publish:

```sh
ingfah agents publish ingfah-agent-mali --revision 31
```

## Edit an agent safely

A revision is a full snapshot, not a patch. If a revision omits `ai_plugin_function_ids`, `phone_tools` or `text_chat_tools`, it has no tool links. Publishing it unbinds every tool from the live agent. If it omits `voice_id`, publishing removes the voice. `agents revise` exists so that you never build that body by hand.

```sh
ingfah agents revise mali --set-file ai_instruction_task=task.md
ingfah agents revise mali --set name=Malee --add-tool plugin:12 --remove-tool phone:5
ingfah agents revise mali --from-file edits.json --dry-run
```

`agents revise` runs these steps:

1. It reads the agent and its newest revision that is not deleted. If that revision is an unpublished draft, it warns you that it is building on the draft.
2. It copies every field of that revision into the create body. It maps `greeting_message` to `ai_greeting_message` and `voice.id` to `voice_id`, and it takes the tool ids from the agent, because a revision read does not return them. It then applies your edits. Tool lists change only through `--add-tool` and `--remove-tool`.
3. It refuses the body if a bound tool is missing, if a tool list is absent, or if a flow node names a tool that the body does not link. `--allow-tool-drop` overrides this check. `ingfah api POST /client/agents/SLUG/revisions` runs the same check.
4. It shows a field-by-field diff (prompt text as a unified diff), asks for confirmation, and creates the draft.
5. It reads the new revision back and compares every field it sent. A mismatch exits 1 and tells you not to publish.

Then publish by id:

```sh
ingfah agents publish mali --revision 18
```

`agents publish` refuses a revision that is not the newest, because someone may have drafted after you. It also refuses a revision that would unbind a tool that is bound now, such as a draft made in the dashboard without its tools; pass `--allow-tool-drop` when that is intended. The prompt lists any change to tool links or voice. After publishing, it reads the agent again and exits 1 if the bound tools or the voice differ from the revision's.

## Exit codes

| Code | Meaning |
|---|---|
| 0 | Success |
| 1 | API or runtime error (the message, hint, and any `data` the error carries go to stderr) |
| 2 | Usage error |
| 3 | Refused by a guard: route not in the registry, tool drop, missing confirmation, or a file that already exists |

## Development

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

The tests use wiremock and never call the real API. They set `INGFAH_TEST_DISABLE_KEYRING`, so the tests never read or write the system keyring. After you vendor a new spec, run `cargo test -p ingfah-client --test spec`. It fails when an operation is missing from the registry or when its scope differs.

## Not covered

The CLI does not cover the knowledge base (JWT only), the backoffice API, or the Speech API (`speech-api.ingfah.ai`, which uses a separate key).
