# Signet vs Glean — Sales Battle Card

**Audience**: Account execs, sales engineers, field CTOs running competitive deals against Glean.
**Posture**: Direct, numbers-first, honest about where Glean is still strong. We win on price, sovereignty, DX, and openness — not on brand.

---

## 30-Second Pitch

> "Glean is expensive, opaque, and closed. Signet gives you the same context-layer capabilities — enterprise search, agentic tooling, permission-preserving RAG, MCP-native integration — as an open-core platform you can self-host, inspect, and extend. Same feature ceiling at roughly one-third the fully-loaded cost, with a free tier your team can try today."

If you only say three things, say:

1. **Apache-2.0 engine, self-hostable.** Your data never leaves your VPC. Compliance scope doesn't expand.
2. **Transparent published pricing.** No paid POC. No 10% support fee. No 7–12% renewal hikes.
3. **MCP-native from day one.** Every schema becomes an MCP server automatically; works with Claude, ChatGPT, Cursor, Gemini, and your own agents without custom integration.

---

## Pricing Takedown (The Hardest Evidence)

| Cost item | Glean | Signet |
|-----------|-------|--------------|
| Base license (per user/month, search) | $45–50 | Team $39 · Business $69 · Enterprise custom |
| AI add-on (per user/month) | +$15 (bundled in newer contracts) | Included |
| Minimum contract | $50–60K ARR, typically 100+ seats | None below Team; Enterprise floor $60K |
| Paid POC | Up to $70K, sandboxed (no real data) | Free 30-day self-hosted trial, real data |
| Mandatory support fee | ~10% of ARR | Included |
| Annual renewal increase | 7–12% unless capped in writing | Price locked for multi-year |
| Infrastructure (cloud) | ~$120K+ annual on mid-sized deploys | Self-host on your existing infra, or managed cloud at published rates |
| Dedicated admin FTE | Typically required ($80–120K) | Not required (CLI + IaC) |
| **Fully-loaded TCO, 200-user mid-market** | **$350K–$480K/yr** | **$120K–$180K/yr** |

**Proof source**: every major Glean review article (GoSearch, Fritz AI, Cybernews, Onyx, Thunai, Kore) cites these figures from buyer reports in late 2025 / early 2026. Bring printouts to the pricing meeting.

---

## Feature Matrix

| Capability | Glean | Signet | Notes |
|---|:---:|:---:|---|
| Enterprise search across 50+ apps | ✅ | ✅ | 60+ connectors at GA vs Glean's 100+; we ship the connector SDK |
| Permission-inherited retrieval | ✅ | ✅ | We run permission predicates as compiled WASM — auditable |
| AI assistant / chat UX | ✅ | ✅ | Reference console; replaceable — use your own |
| Agent building | ✅ (Glean Agents) | ✅ (via MCP) | MCP-native — works with any agent framework, not a proprietary builder |
| **Self-host / air-gap** | ❌ Cloud only | ✅ | Dealbreaker for regulated verticals |
| **Open source core** | ❌ Proprietary | ✅ Apache-2.0 | No lock-in, auditable, forkable |
| **Transparent pricing** | ❌ Sales-quote only | ✅ Published | Every review article complains about Glean's opacity |
| **Free tier** | ❌ None | ✅ Community | Self-service entry point |
| **Free POC** | ❌ $70K paid | ✅ Shadow mode | 30-day real-data evaluation at zero cost |
| **MCP server auto-generation** | ⚠️ Supports MCP clients | ✅ Generates servers per schema | We expose your data to any MCP client; Glean consumes |
| Permission-aware search | ✅ | ✅ | |
| SSO (SAML/OIDC) | ✅ | ✅ Business tier | |
| SOC 2 Type II | ✅ | ✅ (cloud); self-host inherits customer's scope | |
| HIPAA BAA | ✅ | ✅ Enterprise | |
| FedRAMP | ⚠️ In progress | ⚠️ Roadmap | Neither has it today |
| Typed context schemas (CSL) | ❌ | ✅ | Unique — nothing comparable exists |
| Grounded answer receipts | ⚠️ Citations | ✅ Signed provenance | Full chain-of-custody for every answer |
| Multi-region deployment | ✅ | ✅ Business+ | |
| Custom connector SDK | ⚠️ Limited | ✅ First-class | Rust SDK, TS SDK, source-included |
| Model-agnostic (BYOK) | ✅ Model Hub | ✅ | |

**The two rows that usually close the deal:** self-host + transparent pricing. Nothing else meaningful is hostile-different; these two are.

---

## Objection Handling

### "Glean is the category leader. You're unproven."

> "Glean was also unproven in 2019 when Netflix picked them. Today Netflix runs Onyx — an MIT-licensed open-source context platform — in production. Ramp, Thales Group, and others have followed. Open-source and enterprise-grade aren't opposites; they're where serious companies are going because they don't want to repeat the vendor-lock-in pain they had with prior generations of software. We're offering the same trajectory: inspectable code, self-hostable, no hostage pricing."

### "We need 100+ connectors and you have 60."

> "We ship 60 at GA — covering every system in the Glean top-100 that actually matters to more than a handful of customers. The remaining 40 in their catalog are long-tail and typically unmaintained. What we do that Glean doesn't: ship the connector SDK so your engineers can build anything missing in roughly two days. You get the connector as source, maintained by you. And we have a connector marketplace planned for third-party contributions. In 18 months we'll have more than Glean — and they'll be the ones you actually use."

### "Switching is too risky."

> "Shadow mode. Run Signet alongside Glean for 30 days on the same data. We replay every query against both, emit a delta report showing retrieval quality, latency, and cost. Keep Glean if we lose. If we win — and we expect to on latency and sovereignty, and tie or beat on retrieval quality — you cut $200K+ off next year's budget. The risk is asymmetric in our favor."

### "Your company is small. What if you disappear?"

> "The engine is Apache-2.0. The connector SDK is Apache-2.0. The CLI, the console, the SDKs are all Apache-2.0. If we disappear tomorrow, you keep running. Try that with Glean — the day they raise their next round or change their pricing model, you have no escape hatch. Open-source is the disaster-recovery story enterprise software has always needed and never had."

### "Glean has better AI / better search quality."

> "Prove it. Shadow mode gives you the comparison. What we can tell you now: Glean's retrieval is good; we tune the same dense + sparse + reranker stack the industry has converged on, plus we're model-agnostic so you can use the best frontier model on every query instead of whatever Glean's Model Hub has prioritized this quarter. On latency, we win — Rust engine vs their Python retrieval — and latency drives perceived quality."

### "We already have LangChain / LlamaIndex / a custom RAG stack."

> "Then you know the real cost: 2–4 engineers maintaining brittle connectors, ad-hoc permission filtering you're not 100% sure is correct, no audit receipts when compliance asks, no MCP gateway, a different retrieval configuration in every team. Signet is what happens when you promote that stack from framework-level to platform-level. Keep LangChain for orchestration — we feed it. We replace the plumbing, not the brains."

### "Our security team will never approve an open-source project."

> "Two angles. One: the Apache-2.0 license actually makes us more auditable than Glean — your security team can read every line of the retrieval pipeline, the policy engine, and the gateway. With Glean, you get a SOC 2 report and a hope. Two: for regulated deployments, we ship the same Enterprise Edition with FedRAMP-targeted hardening, HIPAA BAA, and air-gapped support, on the same open core. You get the best of both."

### "Glean's renewal cap is only 5% now."

> "Ask to see that in writing with no out clauses. Then ask what happens to your POC if you don't sign. Then ask what the base price resets to if you ever miss a renewal window. The 7–12% is the public-buyer-report baseline; your 5% is probably this year's promotional rate."

---

## Per-Persona Talk Tracks

### CIO / VP IT

**Lead with**: TCO, control, strategic flexibility.

> "You've built a 50-tool SaaS portfolio. The last thing you need is a 51st tool that owns your data and raises prices 10% a year. Signet is infrastructure you control — runs in your VPC, speaks every AI standard, and costs one-third what Glean does fully loaded. When you're asked about AI strategy next board meeting, 'we built a governed context layer in-house at a fraction of the cost' is a better story than 'we wrote a $400K check to Glean.'"

**Anchor metric**: $350K–$480K/year → $120K–$180K/year for the same 200-user deployment.

### VP Engineering / Head of AI

**Lead with**: DX, composability, MCP.

> "Your engineers are already wiring up MCP servers for internal tools. Glean doesn't help with that — it's a walled garden. Signet generates MCP servers per schema automatically. Your agents — whether built on Claude, LangGraph, CrewAI, or internal — get typed tools for every data source with permissions baked in. You stop solving the same RAG problem over and over. Every team's agent gets the same governed context layer. That's where a context platform actually earns its price."

**Anchor artifact**: show a live CSL schema compiling to an MCP server in under 30 seconds.

### CISO / Security

**Lead with**: sovereignty, audit, lineage.

> "Glean indexes your data into their cloud. That expands your compliance scope — every SOC 2, every ISO audit, every customer SIG now has to ask about Glean's posture. With Signet self-hosted, your data never leaves your environment. Every answer the system produces ships with a signed receipt: caller, sources, policies applied, timestamps. When your legal team asks 'how did the AI know that?', you have a provable chain of custody. Glean gives you citations; we give you receipts."

**Anchor artifact**: show a receipt JSON with the full lineage graph.

### Procurement

**Lead with**: transparency, predictability, no hidden costs.

> "Here's our published price list. Here's the free tier. Here's the self-hosted deployment option. Here's the Enterprise contract template — no mandatory support fee, no auto-renewal unless you sign it, price-locked for the term. What you see is what you pay. Now ask your Glean rep for the same. If they'll even give you a contract template before you sign an NDA, compare the two carefully — the support fee clause alone is 10% ARR that you will not get rid of at renewal."

**Anchor artifact**: side-by-side contract template comparison. Glean's contract has a ~10% mandatory support clause that every renewal forum on Reddit complains about.

### End Users (Quieter Signal)

**Lead with**: speed, model choice.

> "Retrieval latency on a Rust engine is 30–60% faster than on a Python stack at p99. For a user typing a query, that's the difference between 'this tool is slow' and 'this tool is fast.' And your org picks the model that answers — not whatever Glean's Model Hub has wired in this quarter."

---

## The Shadow-Mode POC (How to Beat Glean's $70K Paid POC)

**The pitch:**

> "Glean's POC costs $70K and runs against sandboxed fake data. Ours is free, runs on your real data, and you don't commit to anything. Here's the 30-day plan."

**Week 1 — Stand up.** Signet Community Edition self-hosted, or Developer Cloud free tier. Three connectors: GitHub, GDrive, Slack (customer-chosen). CSL schema per major data source. Gateway live. Permissions inherited. Total elapsed time: half a day, one engineer.

**Week 2 — Query mirror.** Plumb a small proxy in front of the customer's existing Glean instance that fans every query to both systems. Record results, latencies, and (optionally) user preference click-throughs.

**Week 3 — Delta report.** Compare retrieval quality (nDCG@10 on a handful of labeled queries the customer's team produces), latency distribution, and cost per query. Ship a 10-page report with real numbers.

**Week 4 — Decision.** Customer either migrates off Glean at next renewal, or we leave cleanly with no contract to unwind. The free tier has no expiration.

**Why this works:**
- The asymmetry is unmistakable: a free, real-data evaluation on one side; a paid, sandboxed evaluation on the other.
- The delta report is objective — we're confident enough in our retrieval to publish the comparison.
- Procurement loves it because it's zero-risk.
- Security loves it because no data leaves the customer's environment.
- Engineering loves it because it's a real technical evaluation, not a demo.

**The customer does need to do real work here** — label queries, stand up the proxy, run for four weeks. Be honest about this. But it's the same work a serious Glean evaluation would require, and it costs $70K less.

---

## Common Glean FUD and Responses

| They say | You respond |
|----------|-------------|
| "We have 5 years of production hardening" | "And we have zero years of legacy architecture to unwind. The context-platform category didn't exist 5 years ago. Everyone starts from the MCP era now." |
| "Open source can't match enterprise security" | "Every major enterprise runs on open-source operating systems, databases, and cloud infrastructure. The open-source vs enterprise framing is 2012 FUD. Today's question is: who controls the code and who controls the data? We answer both 'you.' Glean answers 'us.'" |
| "You'll never get to their connector count" | "The long tail of connectors is a maintenance burden, not a moat. Our connector SDK is Apache-2.0 and takes two days to build a new connector. When your customer needs SAP, we'll have it on the Enterprise tier; when they need Pendo, you build it in an afternoon." |
| "We have Glean Protect for prompt injection" | "Every serious platform has this now. Ours is WASM-isolated policy with PII redaction, caller-identity filtering pre-LLM, and receipts post-LLM. The threat model is public; the defenses are standardized." |
| "We have customer references at your target accounts" | "So do we, in the OSS space. Onyx runs at Netflix, Ramp, and Thales. Dust runs at mid-market companies across Europe. Our named enterprise wins are [names as they accumulate]. The category's reference list has grown beyond just Glean." |
| "You're cheap because you're cutting corners" | "We're cheap because the economics of open-core shift cost from customer to vendor-margin. Glean's 85% gross margin pays for their sales team. Our open-core model means we sell to customers who already self-qualified. The savings flow to you." |

---

## Honest Weaknesses (Acknowledge These)

Don't try to fake equivalence. Buyers notice. When these come up:

**1. Polish and battle-testing.**

> "Glean has had five years and thousands of customers to polish their search UI. We're newer and some rough edges will show. Our counter: the surface area that matters for enterprise — the retrieval engine, the policy layer, the MCP gateway — we've built from scratch for 2026 primitives. The UI is a thin layer on top, and you can replace it with your own anyway."

**2. Enterprise brand recognition.**

> "Glean shows up on the 'enterprise AI' Gartner chart. We don't yet. If your procurement or board requires Gartner-blessed vendors, this is a timing conversation — we'll be there in 18–24 months. If your team evaluates on capability and cost, you're evaluating now."

**3. Onboarding hand-holding.**

> "Glean has a paid implementation services arm with hundreds of consultants. We have a smaller SE team and published docs. For complex enterprise rollouts, we partner with implementation firms; for everything else, our CLI stands up the stack in one command and our docs are public. Different models. Yours to choose."

**4. Some premium connectors are Enterprise-tier-only.**

> "SAP, Oracle EBS, Workday, NetSuite are in our Enterprise tier under a commercial license. If you need these, you're in the tier where Glean would also land you. If you don't, Team and Business cover every connector Glean ships for the mid-market."

**5. No FedRAMP today.**

> "Neither does Glean at the moderate level as of early 2026. If your timeline requires FedRAMP *today*, we're not for you. If it's a 2027 requirement, we're tracking the same certification path."

---

## Email Templates

### Cold outreach — to a Glean customer at renewal

> Subject: Context layer at 1/3 the cost
>
> [First name],
>
> If you're staring down a Glean renewal with a 7–12% price hike and wondering whether you still need the full $350K+ fully-loaded spend — worth a 20-minute look at Signet.
>
> Apache-2.0 engine you can self-host. MCP-native from day one. Permission-inherited retrieval, grounded receipts, SSO, audit. Team tier is $39/user vs Glean's ~$65 all-in. Free 30-day shadow-mode evaluation against your real data and your existing Glean deployment.
>
> No paid POC. No lock-in. Here's the spec if you want to evaluate before a call: [link].
>
> — [Name]

### Warm outreach — to an engineering leader

> Subject: Replacing your LangChain scaffolding with something you'd actually pay for
>
> [First name],
>
> Saw your team's post about standing up RAG pipelines across [Jira/GDrive/Slack] with LangChain. You're 3 engineers deep and still don't have permission-safe retrieval, right?
>
> Signet is the platform layer that slot fills. Rust engine, Apache-2.0, MCP-native, runs on your infra. One CSL file per data source compiles to a typed MCP server with permissions baked in. You keep LangChain for orchestration — we replace the plumbing.
>
> `signet dev` has a working stack on your laptop in 60 seconds if you want to kick the tires: [link].
>
> — [Name]

### Follow-up after a Glean-vs-Signet demo

> Subject: Re: Signet — three things I didn't want to gloss over
>
> [First name],
>
> Three honest follow-ups from the demo:
>
> 1. Glean *is* more polished on the end-user search UI today. We're closer than you might think on retrieval quality and well ahead on latency, but the Glean product has had five years to sand down rough edges in the consumer chat experience. If that's where your users live, factor it in.
>
> 2. The 30-day shadow-mode eval is the right way to settle this — I don't want to win on talking points if your actual queries favor them. My offer: I'll have an SE stand up the eval harness with your team next week. Zero cost, you keep Glean running the entire time.
>
> 3. If you want to go deeper on the receipt / audit story before that, here's a sample receipt JSON for one of our reference deployments: [attach]. Security teams usually have the most questions about this; happy to schedule a 15-minute technical deep-dive.
>
> — [Name]

---

## Proof Points to Accumulate

Start tracking these from the first paying customer:

- Number of retrieval queries served per day (aggregate, across public deployments)
- p50 / p99 retrieval latency (ours vs Glean's — from customers who can share)
- GitHub stars on the public repo
- Community connector contributions merged
- Named enterprise wins (logo list, with permission)
- Named migrations *from* Glean (with anonymized case studies at first; named later)
- Average deployment time (self-host: target < 1 day; managed: target < 1 hour)
- Support response time (publish the percentile)

Every sales deck refresh, update these numbers.

---

## Internal Qualifying Questions (Before You Pitch)

Don't waste cycles on accounts where Glean is the right answer. Ask early:

1. **Is this a regulated industry with data-sovereignty requirements?** If yes → strong fit. If no, other factors matter more.
2. **What's the current AI / context budget?** If sub-$50K/year → they're not buying Glean either; pitch Developer Cloud and Team tier. If $200K+ → full enterprise pitch.
3. **Who is the decision-maker?** If CIO-led and procurement-heavy → lead with TCO. If engineering-led → lead with DX and MCP. If security-led → lead with sovereignty and receipts.
4. **Do they already have an AI platform team?** If yes → we're infrastructure for them; great fit. If no → they need more hand-holding; Glean's SI partnerships may match better short-term.
5. **Is this a greenfield AI rollout or a Glean displacement?** Greenfield: easier sell, no switching cost. Displacement: shadow-mode POC mandatory.

If accounts fail 3+ of these, disqualify. Better to close fewer great-fit deals than to grind on bad-fit ones.

---

## One-Line Summaries by Scenario

- **Displacing Glean at renewal**: "Same capabilities, one-third the TCO, open source, self-host optional."
- **Greenfield AI rollout, mid-market**: "Skip the LangChain spaghetti; start with a real platform."
- **Regulated vertical**: "Your data never leaves your VPC. Ever."
- **Engineering-heavy org**: "MCP-native. CSL. CLI. API. SDK. Stop writing RAG glue."
- **Agent-builder buyer**: "Every schema is an MCP server. Your agents plug in; they don't integrate."

*End of battle card. Update quarterly as pricing, features, and competitive posture evolve.*
