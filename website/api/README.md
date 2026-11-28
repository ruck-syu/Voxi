# Voxi feedback

Voxi can send requests for commands it did not understand and reports about wrong results. GitHub tracks each in its own public checklist.

## How it works

1. `POST /api/command-request` validates the command and note, then posts a comment to the newest open issue labelled `command-requests`.
2. **Wrong result? → Save report** saves the full report locally. **Review report to send → Send this report** posts the public copy to `/api/wrong-result`. The function validates the command, outcome kind, planned action kinds and counts, note, and app version. Both the app and the function reject path-like text, including common filename forms. Only user-entered text and the displayed summary fields can be sent. The function posts a comment to the configured, locked `wrong-results` issue.
3. The **Voxi feedback** workflow (`.github/workflows/command-requests.yml`) folds comments from the configured author into the appropriate checklist. A command request matches an unticked command regardless of case or spacing. A wrong-result report matches by canonical command text and outcome kind. On a match, the count increases and the submission details are kept. A repeat of a ticked entry starts a fresh checkbox.
4. Both functions append comments. One workflow run at a time edits the issue bodies and deletes verified comments. Each run reads both queues. A scheduled run every six hours picks up any comments a failed run left behind. The issue body records comment IDs so a retry can skip reports it already folded.
5. At about 60,000 characters, the workflow creates and locks a continuation issue and links it from the previous issue. Each checklist can span up to 20 issues. Keep the configured **Wrong results** issue open as the queue even when the checklist continues elsewhere.

The endpoints share validation and submission code in `_requests.js`. The app and endpoint share the report schema in `_wrong-result-schema.mjs`.

## Owner setup

1. Use the existing fine-grained `GITHUB_TOKEN_COMMAND_REQUESTS` in the Vercel website project. It needs **Issues: Read and write** on `ruck-syu/Voxi` and must belong to an account that can comment on locked issues. For another repo, set `COMMAND_REQUESTS_REPO` to `owner/repo`. For a token owned by another account, set the GitHub repository variable `COMMAND_REQUESTS_AUTHOR` to that account's login. It applies to both feedback types.
2. Once the updated workflow is on the default branch, run **Voxi feedback** from the Actions tab. It creates and locks missing tracking issues and keeps the existing command-request checklist.
3. Pin **Wrong results**. Keep **Command requests** pinned.
4. In Vercel, set `WRONG_RESULTS_ISSUE_NUMBER` to the number of the original **Wrong results** issue, without the `#`. Redeploy the website with the updated function and environment variables.
5. From the updated app, save a report with sample text, review it, and send it. Check that the public checklist gets the report and that the workflow deletes the queued comment.

Tick a command when it ships or a wrong-result report when it is fixed. A new wording for an existing action can ship through the vocabulary pack (`vocab/vocab.json`).

## Limits

- **Per IP:** five submissions every ten minutes per endpoint. IPv6 addresses are grouped by /64.
- **Per function instance:** 60 submissions every ten minutes. A cold start resets the in-memory limits. For a hard limit, add Vercel Firewall rules for the feedback endpoints.
- **Per workflow run:** at most 100 comments per checklist.
- **Issue edits:** GitHub supports unconditional issue body updates. The workflow reads immediately before writing and checks the receipt IDs afterwards.
- **Logging:** the functions and workflow omit tokens and submission text from logs.

From the repository root, run `node --test .github/scripts/command-requests.test.mjs` for the endpoint and folding tests. From `app/`, run `node --test scripts/wrong-report.test.mjs` for the client sanitizer.
