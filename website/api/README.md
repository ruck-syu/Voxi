# Command requests

People can send a command Voxi didn't understand from the app. It ends up as a line in one public checklist issue, which you tick when the command ships.

## How it works

1. `POST /api/command-request` (`command-request.js`) validates the command and note. It then adds them as a comment on the newest open issue labelled `command-requests`. The function only ever adds comments, so two requests at once can't overwrite each other or your ticks.
2. The **Command requests** workflow (`.github/workflows/command-requests.yml`) runs when such a comment appears. It folds every queued comment into the checklist and deletes the folded comments:
   - A repeat of an unticked command bumps its `(+N)`.
   - A repeat of a ticked command adds a fresh line.
   - Notes go on an indented line.
   - Everything else you wrote in the issue is kept.
3. Runs never overlap, and each run folds everything still queued, so cancelled or failed runs lose nothing. A scheduled run every six hours picks up anything a failed run left behind. Folded comment ids are recorded in the issue body, so a retry never counts a request twice.
4. At about 60,000 characters the workflow opens "Command requests (2)", and so on. It links the new issue from the old one. It stops at 20 issues.

Shared validation and formatting live in `_requests.js`. Vercel doesn't turn files starting with `_` into functions.

## Owner setup

1. Create a fine-grained GitHub personal access token scoped to `ruck-syu/Voxi`, with **Issues: Read and write** as its only permission.
2. In Vercel, open the website project and add the token under **Settings → Environment Variables** as `GITHUB_TOKEN_COMMAND_REQUESTS`. `COMMAND_REQUESTS_REPO` overrides the default repo, `ruck-syu/Voxi`. Redeploy.
3. Run the **Command requests** workflow once from the Actions tab. It creates the label and the "Command requests" issue.
4. Pin the issue, then **lock its conversation**. Locking stops anyone else from commenting. The token still can, because you own the repo.
5. If the token belongs to an account other than the repo owner, set the repository variable `COMMAND_REQUESTS_AUTHOR` to that login. Only comments from that account are folded.

Tick a line when its command ships. A new wording for an existing action ships through the vocabulary pack (`vocab/vocab.json`) without an app release. A new kind of action needs one.

## Limits

- **Per IP:** five requests every ten minutes for each IP address. An IPv6 address counts as its whole /64 network.
- **Per function instance:** 60 requests every ten minutes in total.
- **Resets:** both limits live in memory, so a cold start resets them. For a hard limit, add a rate-limit rule for `/api/command-request` in Vercel Firewall.
- **Per run:** the workflow folds at most 100 requests. The rest wait for the next run.
- **Overwrites:** GitHub can't make a conditional issue update. The workflow reads the issue immediately before writing it and verifies the write afterwards. If you edit the issue at that same moment, the run fails and the next run retries the request.
- **Logging:** neither the function nor the workflow logs the token or the request text.

Tests: from the repository root, run `node --test .github/scripts/command-requests.test.mjs`.
