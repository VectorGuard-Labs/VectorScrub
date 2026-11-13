# Pilot playbook (human-assisted removals)

What a person does, with the tools that exist today. Nothing here is a promise to customers beyond what the Terms say. This is the internal procedure.

## Operating model for the pilot

**Guide-only.** The customer's hands stay on their own keyboard and their own inbox. A person tells them what to do, in what order, and records what happened. We do not hold customers' identity details in a form a person can read: there is deliberately no operator route that releases them, and you should not copy details out of the database. If the pilot shows that submitting on a customer's behalf is needed, build a scoped, audited release for that first. Do not work around the lack of one.

Never: ask for an email password or a code from their inbox; ask them to get past a site's verification; take ID documents by ordinary email; claim to be a legal representative or to exercise statutory rights for them.

## Who to enroll

Keep the number small (about 20) and include different kinds of participant. Tag each one with a neutral cohort letter (`a`, `b`, `c`) and keep what the letters mean in a private note outside the database. Do **not** enroll anyone whose safety depends on this working (for example someone facing a credible threat) until you have: legal review of the Terms and Privacy Policy, MFA and access controls on every admin path, and an incident-response procedure. The product has no security certification and a 45-day verification window; a person at risk needs more than a pilot.

## Steps for each customer

0. **Before anyone joins.** Set `MAX_ACCOUNTS` to the size of the pilot, set `PILOT_PAYMENT_URL` to a one-time $1 Stripe payment link (not a subscription), and keep `SHOW_PAID_PLAN` off. When a payment arrives, match it to the account by email, tag the account (`pilot cohort`), and confirm the place by email. The $1 is a commitment device: the scan and three opt-outs are also in the free account, and the pilot adds a person's attention. Say that plainly. The scorecard shows who agreed to an interview or to being asked about a story; never use either without asking again.
1. **Intake.** They register, accept the Terms, enter their identity profile, and run a scan. Log the time: `vectorscrub pilot log-time <email> intake <minutes>`.
2. **Assessment.** Open the case: `vectorscrub pilot case <email>`. It lists their listings and the next action for each, with no names or links. Sites we could not read are in their dashboard coverage panel. Check the detected listings with the customer: is this you? Log `review` time.
3. **Authorization.** The customer approves each listing in the dashboard. Nothing is sent before that.
4. **Removal.** Automated where the site allows it. Where it is blocked, the case shows "Manual action required". Walk the customer through the opt-out (screen share is fine, hands on their keyboard). They then press "Record what I did" with the date and whether they clicked the site's email. Log `submission` time. Close help items with `vectorscrub assistance finish <id> <minutes>`.
5. **Verification.** Re-checks run on the schedule (about days 3, 7, 14, 30; closed at 45). Do not tell a customer a listing is gone because the page would not load. A result you can only call "inconclusive" is reported as inconclusive. Log `verification` and `followup` time.
6. **Report.** The customer's report page (Dashboard → Report) is printable and carries no names or links. Send them to it, and log `report` time.

## Reading the numbers

`vectorscrub pilot scorecard` prints the funnel, overall and per cohort. Read the stages separately:

- **Submitted is not removed.** The headline is the *verified rate* (verified ÷ submitted), not the submission count.
- **Verification coverage** is how many submitted requests have had at least one clear check. Low coverage means the data is thin, not that removals failed.
- **Manual path** (needed → reported → verified) is the funnel for opt-outs done by hand, including listings customers added themselves. If many are reported but few verified, the instructions or the verification are the problem, not the customer.
- **Labor** is the minutes you logged plus help-request minutes. The scorecard shows minutes per participant; compare it with what you could charge.
- A 30-day window is shorter than the 45-day verification period. Report unfinished requests as awaiting a result and keep following them.

The scorecard cannot tell you match accuracy, satisfaction or willingness to pay. Ask in an interview at the end, and write the answers down per participant.

## Security habits for the operator

- Work from a machine with disk encryption and a screen lock. Do not paste output into chat or email.
- `pilot case` and `assistance contact` are written to the audit log. Use `contact` only to write to a customer who asked.
- Erase accounts on request with `vectorscrub erase-account <email>` after confirming the request comes from the account's owner (reply to the account's own address).
- If you suspect any exposure of customer data, stop, note the time, and follow your incident procedure. Tell affected customers plainly.

## Commands

```
vectorscrub pilot scorecard
vectorscrub pilot cohort <email> <a|b|c>
vectorscrub pilot log-time <email> <intake|review|submission|followup|verification|report|other> <minutes>
vectorscrub pilot case <email>
vectorscrub assistance list | contact <id> | start <id> | finish <id> <minutes> | decline <id> | summary
vectorscrub erase-account <email>
```
