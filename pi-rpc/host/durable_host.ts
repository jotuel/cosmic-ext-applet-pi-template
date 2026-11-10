import { createHash } from "node:crypto";
import { mkdirSync, existsSync } from "node:fs";
import { homedir } from "node:os";
import { basename, join, resolve } from "node:path";
import { createInterface } from "node:readline";

import { BACKGROUND_CONTEXT } from "@earendil-works/chord/context";
import { clampThinkingLevel, createModels } from "@earendil-works/pi-ai";
import { fauxAssistantMessage, fauxProvider, fauxThinking, fauxText } from "@earendil-works/pi-ai/providers/faux";
import {
	createRegistry,
	defineExtension,
	Harness,
	InboxDoc,
	LiveDoc,
	MemoryStorage,
	section,
	UsageDoc,
	watchEvents,
} from "@earendil-works/pi-durable";
import { NodeExecutionEnv } from "@earendil-works/pi-durable/env/node";
import { openNodeSqliteStorage } from "@earendil-works/pi-durable/storage/sqlite/node";
import { CodingTools } from "@earendil-works/pi-durable/tools";

const context = BACKGROUND_CONTEXT;

interface CliOptions {
	workingDir: string;
	sessionDir?: string;
	noSession: boolean;
	provider?: string;
	model?: string;
}

function parseCliArgs(argv: string[]): CliOptions {
	let workingDir = process.cwd();
	let sessionDir: string | undefined;
	let noSession = false;
	let provider: string | undefined;
	let model: string | undefined;

	for (let i = 0; i < argv.length; i++) {
		const arg = argv[i];
		if (arg === "--working-dir" && i + 1 < argv.length) {
			workingDir = resolve(argv[++i]);
		} else if (arg === "--session-dir" && i + 1 < argv.length) {
			sessionDir = resolve(argv[++i]);
		} else if (arg === "--no-session") {
			noSession = true;
		} else if (arg === "--provider" && i + 1 < argv.length) {
			provider = argv[++i];
		} else if (arg === "--model" && i + 1 < argv.length) {
			model = argv[++i];
		} else if (arg === "--mode" && i + 1 < argv.length) {
			i++; // consume mode value if passed
		}
	}

	return { workingDir, sessionDir, noSession, provider, model };
}

function defaultSessionsDir(): string {
	const xdgData = process.env.XDG_DATA_HOME || join(homedir(), ".local", "share");
	return join(xdgData, "cosmic-ext-applet-pi", "sessions");
}

function sessionSqlitePath(workingDir: string, sessionDir?: string): string {
	const baseDir = sessionDir ?? defaultSessionsDir();
	mkdirSync(baseDir, { recursive: true });
	const canonical = resolve(workingDir);
	const folder = basename(canonical).replace(/[^a-zA-Z0-9._-]/g, "_") || "root";
	const hash = createHash("sha256").update(canonical).digest("hex").slice(0, 16);
	return join(baseDir, `${folder}-${hash}.sqlite`);
}

function writeJsonLine(record: unknown): void {
	process.stdout.write(`${JSON.stringify(record)}\n`);
}

function extractInputText(content: unknown): string {
	if (typeof content === "string") return content;
	if (Array.isArray(content)) {
		return content
			.map((item) => (item && typeof item === "object" && "text" in item && typeof item.text === "string" ? item.text : ""))
			.join("");
	}
	return "";
}

async function loadModelsAndDefaults(cwd: string): Promise<{
	models: ReturnType<typeof createModels>;
	getAvailableModels: () => Promise<any[]>;
	resolveModelInput: (text: string, providerHint?: string) => Promise<{ provider: string; modelId: string } | undefined>;
	isScopedModel: (ref: { provider: string; modelId: string }) => boolean;
	hasScopedModels: boolean;
	defaultProvider?: string;
	defaultModelId?: string;
	defaultThinkingLevel?: string;
}> {
	const codingAgentCore =
		process.env.PI_CODING_AGENT_CORE_DIR || "/usr/lib/node_modules/@earendil-works/pi-coding-agent/dist/core";
	const servicesPath = join(codingAgentCore, "agent-session-services.js");
	const modelResolverPath = join(codingAgentCore, "model-resolver.js");

	if (existsSync(servicesPath) && existsSync(modelResolverPath)) {
		try {
			const { createAgentSessionServices } = await import(servicesPath);
			const { resolveModelScopeWithDiagnostics, findInitialModel, resolveModelReference } =
				await import(modelResolverPath);

			const { modelRuntime: runtime, settingsManager: settings } = await createAgentSessionServices({
				cwd,
			});

			const enabledPatterns: string[] = settings.getEnabledModels?.() ?? [];
			const { scopedModels } =
				enabledPatterns.length > 0
					? await resolveModelScopeWithDiagnostics(enabledPatterns, runtime)
					: { scopedModels: [] as any[] };
			const scopedModelList: any[] = scopedModels.map((s: any) => s.model);

			const initial = await findInitialModel({
				scopedModels,
				isContinuing: false,
				defaultProvider: settings.getDefaultProvider(),
				defaultModelId: settings.getDefaultModel(),
				defaultThinkingLevel: settings.getDefaultThinkingLevel(),
				modelThinkingLevels: settings.getAllModelThinkingLevels?.(),
				modelRuntime: runtime,
			});

			const resolveModelInput = async (
				text: string,
				providerHint?: string,
			): Promise<{ provider: string; modelId: string } | undefined> => {
				const trimmed = text.trim();
				if (!trimmed) return undefined;
				const usable = (providerId: string) => Boolean(runtime.hasConfiguredAuth?.(providerId));

				// 1. Prefer resolving within enabledModels (scopedModels) when configured
				if (scopedModelList.length > 0) {
					const scopedMatch = resolveModelReference(
						{ text: trimmed, provider: providerHint, thinkingSuffix: "lenient", fuzzy: true },
						scopedModelList,
						usable,
					);
					if (scopedMatch?.status === "resolved" && scopedMatch.model) {
						return { provider: scopedMatch.model.provider, modelId: scopedMatch.model.id };
					}
				}

				// 2. Try available (authenticated) models
				const available = await runtime.getAvailable().catch(() => []);
				if (Array.isArray(available) && available.length > 0) {
					const availMatch = resolveModelReference(
						{ text: trimmed, provider: providerHint, thinkingSuffix: "lenient", fuzzy: true },
						available,
						usable,
					);
					if (availMatch?.status === "resolved" && availMatch.model) {
						return { provider: availMatch.model.provider, modelId: availMatch.model.id };
					}
				}

				// 3. Try full catalog
				const allModels = [...runtime.getModels()];
				const allMatch = resolveModelReference(
					{ text: trimmed, provider: providerHint, thinkingSuffix: "lenient", fuzzy: true },
					allModels,
					usable,
				);
				if (allMatch?.status === "resolved" && allMatch.model) {
					return { provider: allMatch.model.provider, modelId: allMatch.model.id };
				}

				if (trimmed.includes("/")) {
					const [p, m] = trimmed.split("/", 2);
					return { provider: p, modelId: m };
				}
				return {
					provider: providerHint ?? initial.model?.provider ?? settings.getDefaultProvider() ?? "radius",
					modelId: trimmed,
				};
			};

			return {
				models: runtime as unknown as ReturnType<typeof createModels>,
				getAvailableModels: async () => {
					if (scopedModelList.length > 0) {
						return scopedModelList;
					}
					try {
						const avail = await runtime.getAvailable();
						if (Array.isArray(avail) && avail.length > 0) return avail;
					} catch {
						// fallback to all models
					}
					return runtime.getModels();
				},
				resolveModelInput,
				isScopedModel: (ref) =>
					scopedModelList.some((m) => m.provider === ref.provider && m.id === ref.modelId),
				hasScopedModels: scopedModelList.length > 0,
				defaultProvider: initial.model?.provider ?? settings.getDefaultProvider(),
				defaultModelId: initial.model?.id ?? settings.getDefaultModel(),
				defaultThinkingLevel: initial.thinkingLevel ?? settings.getDefaultThinkingLevel(),
			};
		} catch (err) {
			console.error("Warning: failed to load pi-coding-agent services, falling back to pi-ai:", err);
		}
	}

	const models = createModels();
	const faux = fauxProvider({
		models: [{ id: "faux-default", name: "Faux Default", reasoning: true, contextWindow: 128000, maxTokens: 8192 }],
	});
	const delayMs = Number(process.env.PI_DURABLE_FAUX_DELAY_MS ?? 0);
	faux.setResponses([
		async (ctx) => {
			if (delayMs > 0) {
				await new Promise((r) => setTimeout(r, delayMs));
			}
			// Re-queue handler so multiple turns work in faux mode
			faux.appendResponses([
				async (nextCtx) => {
					if (delayMs > 0) {
						await new Promise((r) => setTimeout(r, delayMs));
					}
					return fauxAssistantMessage([
						fauxThinking("Processing request..."),
						fauxText(`Durable response (${nextCtx.messages.length} messages in context)`),
					]);
				},
			]);
			return fauxAssistantMessage([
				fauxThinking("Processing request..."),
				fauxText(`Durable response (${ctx.messages.length} messages in context)`),
			]);
		},
	]);
	models.setProvider(faux.provider);
	return {
		models,
		getAvailableModels: async () => [...models.getModels()],
		resolveModelInput: async (text: string, providerHint?: string) => {
			if (text.includes("/")) {
				const [p, m] = text.split("/", 2);
				return { provider: p, modelId: m };
			}
			return { provider: providerHint ?? faux.provider.id, modelId: text };
		},
		isScopedModel: () => true,
		hasScopedModels: false,
		defaultProvider: faux.provider.id,
		defaultModelId: "faux-default",
		defaultThinkingLevel: "medium",
	};
}

const ALL_THINKING_LEVELS = ["off", "minimal", "low", "medium", "high", "xhigh", "max"] as const;

async function main(): Promise<void> {
	const cli = parseCliArgs(process.argv.slice(2));
	const {
		models,
		getAvailableModels,
		resolveModelInput,
		isScopedModel,
		hasScopedModels,
		defaultProvider,
		defaultModelId,
		defaultThinkingLevel,
	} = await loadModelsAndDefaults(cli.workingDir);

	const registry = createRegistry();
	registry.install(CodingTools);
	registry.install(
		defineExtension({
			name: "cosmic-applet-pi",
			sections: [
				section(
					"preamble",
					() =>
						"You are Pi, a concise and capable AI coding assistant running on the Pi Durable (@earendil-works/pi-durable) runtime inside the COSMIC Desktop panel applet.",
					{ tag: false },
				),
				section("cwd", (input) => input.env?.cwd),
			],
		}),
	);

	let steeringMode: "all" | "one-at-a-time" = "one-at-a-time";
	let followUpMode: "all" | "one-at-a-time" = "one-at-a-time";
	let autoCompactionEnabled = true;
	let autoRetryEnabled = true;
	let sessionName: string | undefined;

	const sessionFile = cli.noSession ? undefined : sessionSqlitePath(cli.workingDir, cli.sessionDir);
	const storage = cli.noSession ? new MemoryStorage() : await openNodeSqliteStorage(sessionFile!);

	const harness = await Harness.open(
		storage,
		{
			models,
			registry,
			settings: {
				get steeringMode() {
					return steeringMode;
				},
				get followUpMode() {
					return followUpMode;
				},
				get compaction() {
					return { enabled: autoCompactionEnabled };
				},
				get retry() {
					return { enabled: autoRetryEnabled };
				},
			},
			env: ({ cwd }) => new NodeExecutionEnv({ cwd: cwd ?? cli.workingDir }),
		},
		context,
	);

	// Resolve initial model
	let initialModelRef: { provider: string; modelId: string } | undefined;
	if (cli.model) {
		initialModelRef = await resolveModelInput(cli.model, cli.provider);
	} else if (defaultProvider && defaultModelId) {
		initialModelRef = { provider: defaultProvider, modelId: defaultModelId };
	}

	const root = await harness.root(context, {
		agent: {
			...(initialModelRef ? { model: initialModelRef } : {}),
			...(defaultThinkingLevel ? { thinkingLevel: defaultThinkingLevel as any } : {}),
			cwd: cli.workingDir,
		},
	});

	// Ensure cwd (and CLI model override or repair of out-of-scope persisted model) is applied
	const currentAgent = await root.agent(context);
	const agentPatch: Record<string, unknown> = {};
	if (currentAgent.cwd !== cli.workingDir) {
		agentPatch.cwd = cli.workingDir;
	}
	if (initialModelRef) {
		const needsModelUpdate =
			!currentAgent.model ||
			Boolean(cli.model && (currentAgent.model.provider !== initialModelRef.provider || currentAgent.model.modelId !== initialModelRef.modelId)) ||
			(hasScopedModels && !isScopedModel(currentAgent.model)) ||
			!models.getModel(currentAgent.model.provider, currentAgent.model.modelId);
		if (needsModelUpdate) {
			agentPatch.model = initialModelRef;
		}
	}
	if (Object.keys(agentPatch).length > 0) {
		await root.configure(agentPatch as any, context);
	}

	async function enrichInboxItems(items: readonly { id: number; mode: string }[]): Promise<any[]> {
		try {
			const inboxDoc = await harness.snapshot(InboxDoc, root.id, context);
			const byId = new Map((inboxDoc?.items ?? []).map((it: any) => [it.id, it]));
			return items.map((item) => {
				const full = byId.get(item.id as any);
				const text = full && "content" in full ? extractInputText(full.content) : undefined;
				return text ? { ...item, text } : item;
			});
		} catch {
			return [...items];
		}
	}

	async function enrichEvent(event: any): Promise<any> {
		if (event.type === "snapshot" && Array.isArray(event.inbox)) {
			return {
				...event,
				inbox: await enrichInboxItems(event.inbox),
			};
		}
		if (event.type === "inbox_update" && Array.isArray(event.items)) {
			return {
				...event,
				items: await enrichInboxItems(event.items),
			};
		}
		return event;
	}

	const eventStream = await watchEvents(harness, root.id, context);
	writeJsonLine(await enrichEvent(eventStream.snapshot));

	eventStream.start(async (events) => {
		for (const ev of events) {
			writeJsonLine(await enrichEvent(ev));
		}
	});

	// Resume any unfinished work left from before applet reload
	harness.resume();

	function serializeModel(m: any): any {
		if (!m) return null;
		return {
			id: m.id,
			name: m.name ?? m.id,
			provider: m.provider,
			reasoning: Boolean(m.reasoning),
			contextWindow: m.contextWindow,
			maxTokens: m.maxTokens,
		};
	}

	async function buildSessionState(): Promise<any> {
		const resolvedAgent = await root.agent(context);
		const live = await harness.snapshot(LiveDoc, root.id, context);
		const inbox = await harness.snapshot(InboxDoc, root.id, context);
		const ctxView = await root.context(context);

		let modelObj = null;
		if (resolvedAgent.model) {
			const found = models.getModel(resolvedAgent.model.provider, resolvedAgent.model.modelId);
			modelObj = found
				? serializeModel(found)
				: {
						id: resolvedAgent.model.modelId,
						name: resolvedAgent.model.modelId,
						provider: resolvedAgent.model.provider,
					};
		}

		return {
			model: modelObj,
			thinkingLevel: resolvedAgent.thinkingLevel ?? "medium",
			isStreaming: Boolean(live?.run),
			isCompacting: Boolean(live?.compactions && live.compactions.length > 0),
			steeringMode,
			followUpMode,
			sessionFile: sessionFile ?? null,
			sessionId: `durable-${root.id}`,
			sessionName: sessionName ?? null,
			autoCompactionEnabled,
			messageCount: ctxView.entries.length,
			pendingMessageCount: inbox?.items?.length ?? 0,
		};
	}

	async function handleCommand(cmd: any): Promise<any> {
		const type = cmd.type;
		switch (type) {
			case "submit":
			case "prompt":
			case "steer":
			case "follow_up": {
				const message: string = cmd.message ?? "";
				const images: any[] = Array.isArray(cmd.images) ? cmd.images : [];
				const content =
					images.length > 0
						? [
								...(message ? [{ type: "text" as const, text: message }] : []),
								...images.map((img) => ({
									type: "image" as const,
									data: img.data,
									mimeType: img.mimeType,
								})),
							]
						: message;

				const whenBusy: "steer" | "followUp" | undefined =
					type === "steer"
						? "steer"
						: type === "follow_up"
							? "followUp"
							: cmd.whenBusy ?? cmd.streamingBehavior ?? "followUp";

				const liveBefore = await harness.snapshot(LiveDoc, root.id, context);
				const wasBusy = Boolean(liveBefore?.run);

				const sub = await root.submit(
					{
						type: "input",
						content: content as any,
						whenBusy,
						...(cmd.requestId ? { requestId: String(cmd.requestId) } : {}),
					},
					context,
				);
				const statusRec = await sub.status(context);
				const disposition =
					statusRec.status === "queued" || wasBusy
						? "queued"
						: statusRec.status === "done"
							? "handled"
							: "started";

				return {
					submissionId: sub.id,
					status: statusRec.status,
					disposition,
				};
			}

			case "abort": {
				await root.abort(context);
				return null;
			}

			case "clear_queue": {
				const inbox = await harness.snapshot(InboxDoc, root.id, context);
				const steering: string[] = [];
				const followUp: string[] = [];
				for (const item of inbox?.items ?? []) {
					if (item.mode === "steer" || item.mode === "followUp") {
						const text = extractInputText(item.content);
						if (item.mode === "steer") steering.push(text);
						else followUp.push(text);
						await harness.abortSubmission(item.id, context, root.id);
					}
				}
				return { steering, followUp };
			}

			case "reset":
			case "new_session": {
				const handoff = typeof cmd.handoff === "string" ? cmd.handoff : undefined;
				await root.reset(handoff, context);
				return { cancelled: false };
			}

			case "get_state": {
				return await buildSessionState();
			}

			case "configure": {
				const patch: Record<string, unknown> = {};
				if ("model" in cmd) patch.model = cmd.model;
				if ("thinkingLevel" in cmd) patch.thinkingLevel = cmd.thinkingLevel;
				if ("cwd" in cmd) patch.cwd = cmd.cwd;
				if ("instructions" in cmd) patch.instructions = cmd.instructions;
				await root.configure(patch as any, context);
				return await buildSessionState();
			}

			case "set_model": {
				const rawProvider = cmd.provider ? String(cmd.provider) : undefined;
				const rawModelId = String(cmd.modelId);
				let target =
					rawProvider && models.getModel(rawProvider, rawModelId)
						? { provider: rawProvider, modelId: rawModelId }
						: await resolveModelInput(
								rawProvider && !rawModelId.includes("/") ? `${rawProvider}/${rawModelId}` : rawModelId,
								rawProvider,
							);
				if (!target || !models.getModel(target.provider, target.modelId)) {
					const fallback = await resolveModelInput(rawModelId);
					if (fallback) target = fallback;
				}
				const provider = target?.provider ?? rawProvider ?? "radius";
				const modelId = target?.modelId ?? rawModelId;
				await root.configure({ model: { provider, modelId } }, context);
				const found = models.getModel(provider, modelId);
				return found
					? serializeModel(found)
					: {
							id: modelId,
							name: modelId,
							provider,
						};
			}

			case "cycle_model": {
				const avail = await getAvailableModels();
				if (avail.length === 0) return null;
				const current = (await root.agent(context)).model;
				const idx = current
					? avail.findIndex((m) => m.provider === current.provider && m.id === current.modelId)
					: -1;
				const next = avail[(idx + 1) % avail.length];
				await root.configure({ model: { provider: next.provider, modelId: next.id } }, context);
				const resolved = await root.agent(context);
				return {
					model: serializeModel(next),
					thinkingLevel: resolved.thinkingLevel,
					isScoped: false,
				};
			}

			case "get_available_models": {
				const avail = await getAvailableModels();
				return {
					models: avail.map(serializeModel),
				};
			}

			case "set_thinking_level": {
				const level = cmd.level;
				await root.configure({ thinkingLevel: level }, context);
				return null;
			}

			case "cycle_thinking_level": {
				const resolved = await root.agent(context);
				const currentLevel = resolved.thinkingLevel ?? "medium";
				const idx = ALL_THINKING_LEVELS.indexOf(currentLevel as any);
				const nextLevel = ALL_THINKING_LEVELS[(idx + 1) % ALL_THINKING_LEVELS.length];
				const modelObj = resolved.model
					? models.getModel(resolved.model.provider, resolved.model.modelId)
					: undefined;
				const clamped = modelObj ? clampThinkingLevel(modelObj as any, nextLevel as any) : nextLevel;
				await root.configure({ thinkingLevel: clamped as any }, context);
				return { level: clamped };
			}

			case "get_available_thinking_levels": {
				return { levels: [...ALL_THINKING_LEVELS] };
			}

			case "set_steering_mode": {
				steeringMode = cmd.mode === "all" ? "all" : "one-at-a-time";
				return null;
			}

			case "set_follow_up_mode": {
				followUpMode = cmd.mode === "all" ? "all" : "one-at-a-time";
				return null;
			}

			case "compact": {
				const ctxBefore = await root.context(context);
				const taskId = await root.compact(cmd.customInstructions ?? undefined, context);
				const settled = await harness.waitForTask(taskId, context);
				const outcome = settled.state.outcome;
				if (outcome.status === "failed" || outcome.status === "faulted") {
					throw new Error(outcome.error.message);
				}
				let summary = "";
				let firstKeptEntryId = "";
				if (outcome.status === "completed") {
					const res = outcome.result as { entryId?: number; submissionId?: number };
					let entryId = res.entryId;
					if (entryId === undefined && res.submissionId !== undefined) {
						const sub = await harness.submission(res.submissionId as any, context);
						if (sub) {
							const waited = await sub.wait(context);
							if (waited.status === "done" && "entry" in waited) {
								entryId = waited.entry as number;
							}
						}
					}
					if (entryId !== undefined) {
						const ctxAfter = await root.context(context);
						const headEntry = ctxAfter.head;
						if (headEntry) {
							firstKeptEntryId = String(headEntry.head ?? headEntry.id);
							const msg = headEntry.model?.[0];
							if (msg) {
								summary = extractInputText(msg.content);
							}
						}
					}
				}
				return {
					summary,
					firstKeptEntryId,
					tokensBefore: ctxBefore.entries.length,
					estimatedTokensAfter: 0,
				};
			}

			case "set_auto_compaction": {
				autoCompactionEnabled = Boolean(cmd.enabled);
				return null;
			}

			case "set_auto_retry": {
				autoRetryEnabled = Boolean(cmd.enabled);
				return null;
			}

			case "abort_retry": {
				await root.abort(context);
				return null;
			}

			case "bash": {
				const resolved = await root.agent(context);
				const env = new NodeExecutionEnv({ cwd: resolved.cwd ?? cli.workingDir });
				let output = "";
				const res = await env.exec(
					["bash", "-lc", String(cmd.command ?? "")],
					{
						onOutput: (chunk) => {
							output += chunk;
						},
					},
					context,
				);
				return {
					output,
					exitCode: res.exitCode,
					truncated: false,
				};
			}

			case "get_session_stats": {
				const usageState = (await harness.snapshot(UsageDoc, root.id, context)) ?? {
					models: {},
					tools: {},
				};
				const ctxView = await root.context(context);
				let input = 0;
				let output = 0;
				let cacheRead = 0;
				let cacheWrite = 0;
				let cost = 0;
				for (const u of Object.values(usageState.models ?? {})) {
					input += u.input ?? 0;
					output += u.output ?? 0;
					cacheRead += u.cacheRead ?? 0;
					cacheWrite += u.cacheWrite ?? 0;
					cost += u.cost?.total ?? 0;
				}
				for (const u of Object.values(usageState.tools ?? {})) {
					input += u.input ?? 0;
					output += u.output ?? 0;
					cacheRead += u.cacheRead ?? 0;
					cacheWrite += u.cacheWrite ?? 0;
					cost += u.cost?.total ?? 0;
				}
				return {
					sessionFile: sessionFile ?? null,
					sessionId: `durable-${root.id}`,
					totalMessages: ctxView.entries.length,
					cost,
					tokens: {
						input,
						output,
						cacheRead,
						cacheWrite,
					},
					usage: usageState,
				};
			}

			case "set_session_name": {
				sessionName = String(cmd.name ?? "");
				return null;
			}

			case "get_messages": {
				const ctxView = await root.context(context);
				return {
					messages: ctxView.messages,
				};
			}

			case "get_commands": {
				return {
					commands: [],
				};
			}

			default:
				throw new Error(`Unsupported command: ${type}`);
		}
	}

	const rl = createInterface({
		input: process.stdin,
		crlfDelay: Infinity,
	});

	rl.on("line", (line) => {
		const trimmed = line.trim();
		if (!trimmed) return;
		let parsed: any;
		try {
			parsed = JSON.parse(trimmed);
		} catch {
			return;
		}
		if (parsed.type === "extension_ui_response") {
			return;
		}
		const id = parsed.id;
		const command = parsed.type ?? "unknown";
		void (async () => {
			try {
				const data = await handleCommand(parsed);
				writeJsonLine({
					id,
					type: "response",
					command,
					success: true,
					data,
				});
			} catch (err: any) {
				writeJsonLine({
					id,
					type: "response",
					command,
					success: false,
					error: err instanceof Error ? err.message : String(err),
				});
			}
		})();
	});

	let shuttingDown = false;
	async function shutdown(): Promise<void> {
		if (shuttingDown) return;
		shuttingDown = true;
		try {
			await eventStream.stop();
		} catch {
			// ignore
		}
		try {
			await harness.close(context);
		} catch {
			// ignore
		}
		process.exit(0);
	}

	rl.on("close", () => {
		void shutdown();
	});
	process.on("SIGTERM", () => {
		void shutdown();
	});
	process.on("SIGINT", () => {
		void shutdown();
	});
}

main().catch((err) => {
	console.error(err);
	process.exit(1);
});
