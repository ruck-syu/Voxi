# Command requests

`POST /api/command-request` adds an explicitly submitted command and optional note to a public GitHub checklist. A repeat of an unchecked command increments its count. A repeat of a checked command creates a new checklist entry. At about 60,000 characters the endpoint creates the next numbered issue and links to it from the previous one.

## Owner setup

1. Create a fine-grained GitHub personal access token scoped to `ruck-syu/Voxi`. Give it **Issues: Read and write** as its only configurable permission.
2. In Vercel, open the Voxi project and add the token under **Settings → Environment Variables** as `GITHUB_TOKEN_COMMAND_REQUESTS`. The repository defaults to `ruck-syu/Voxi` and can be overridden with `COMMAND_REQUESTS_REPO`.
3. Redeploy the website.
4. Pin **Command requests** after the first submission creates it.

Tick an entry when its command ships. New wording can ship through `vocab/vocab.json` once the vocabulary pipeline is available. A new kind of action needs an app release.

## Operation

The endpoint limits each IP to five valid submissions every ten minutes per warm function instance. The in-memory limit resets on cold starts. The function verifies each GitHub write and retries missing updates, but GitHub's Issues API cannot make a conditional body update, so overlapping edits across instances or in the GitHub editor can still race. Recent request receipts are stored in an HTML comment in the issue body to distinguish concurrent counter changes. The function does not log the token or request body.

Run the mocked endpoint tests from the repository root with `node --test app/scripts/command-request.test.mjs`.
