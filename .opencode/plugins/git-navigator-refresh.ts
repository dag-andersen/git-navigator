import type { Plugin } from "@opencode-ai/plugin"

// Set OPENCODE_GIT_NAVIGATOR_DEBUG=1 to write detailed diagnostics to
// /tmp/opencode-git-navigator-plugin.log.

type CmuxIdentify = {
  caller?: { workspace_ref?: string; pane_ref?: string }
}

type CmuxTree = {
  windows?: Array<{
    workspaces?: Array<{
      ref: string
      panes?: Array<{
        ref: string
        pixel_frame?: { x: number; width: number }
        surfaces?: Array<{ ref: string; title?: string; type?: string }>
      }>
    }>
  }>
}

type CmuxPanes = {
  panes?: Array<{
    ref: string
    pixel_frame?: { x: number; y: number; width: number; height: number }
    surfaces?: Array<{ title?: string; type?: string }>
  }>
}

type FileEditedEvent = {
  type: string
  properties?: { file?: string; path?: string; filePath?: string }
}

type PendingNavigation = {
  file: string
  eventCount: number
}

type NavigatorState = {
  socket?: string
  repositoryIdentity?: string
  worktree?: string
  file?: string
}

const DEBOUNCE_MS = 150

type Shell = (strings: TemplateStringsArray, ...values: unknown[]) => {
  quiet(): Promise<{ text(): string }>
}

const log = async ($: Shell, value: unknown) => {
  if (process.env.OPENCODE_GIT_NAVIGATOR_DEBUG !== "1") return
  try {
    await $`printf '%s\n' ${JSON.stringify({ timestamp: new Date().toISOString(), ...(value as Record<string, unknown>) })} >> /tmp/opencode-git-navigator-plugin.log`.quiet()
  } catch {
    // Diagnostics must never make file editing or navigation fail.
  }
}

const timed = async <T>(
  $: Shell,
  stage: string,
  operation: () => Promise<T>,
  details: Record<string, unknown> = {},
) => {
  const started = performance.now()
  try {
    const result = await operation()
    await log($, {
      stage,
      durationMs: Math.round((performance.now() - started) * 100) / 100,
      ...details,
      ok: true,
    })
    return result
  } catch (error) {
    await log($, {
      stage,
      durationMs: Math.round((performance.now() - started) * 100) / 100,
      ...details,
      ok: false,
      error: String(error),
    })
    throw error
  }
}

const runJson = async <T>($: Shell, command: string, ...args: string[]) => {
  const result = await timed($, `cmux.${command}`, () => $`cmux ${command} ${args}`.quiet(), { args })
  return JSON.parse(result.text()) as T
}

const editedPath = (event: FileEditedEvent) =>
  event.properties?.file ?? event.properties?.path ?? event.properties?.filePath

const existingWorktree = async ($: Shell, file: string) => {
  const directory = file.slice(0, file.lastIndexOf("/")) || "."
  const result = await $`git -C ${directory} rev-parse --show-toplevel`.quiet()
  return result.text().trim()
}

const repositoryIdentity = async ($: Shell, file: string) => {
  const directory = file.slice(0, file.lastIndexOf("/")) || "."
  const result = await $`git -C ${directory} rev-parse --path-format=absolute --git-common-dir`.quiet()
  return result.text().trim()
}

const relativeTo = (root: string, file: string) => {
  const prefix = root.endsWith("/") ? root : `${root}/`
  return file.startsWith(prefix) ? file.slice(prefix.length) : file
}

const refreshAdjacentNavigator = async (
  $: Shell,
  directory: string,
  file: string,
  eventCount: number,
  state: NavigatorState,
) => {
  const started = performance.now()
  // Locate the companion navigator without changing cmux focus.
  // Refreshing the navigator is intentionally best effort.
  const surface = process.env.CMUX_SURFACE_ID
  const workspaceId = process.env.CMUX_WORKSPACE_ID
  if (!surface || !workspaceId) {
    await log($, { stage: "missing-cmux-environment", surface, workspace: workspaceId })
    return
  }
  const identify = await runJson<CmuxIdentify>($, "identify", "--workspace", workspaceId, "--surface", surface)
  await log($, { stage: "identify", identify })
  const callerPane = identify.caller?.pane_ref
  const workspaceRef = identify.caller?.workspace_ref
  if (!callerPane) {
    await log($, { stage: "missing-caller-pane" })
    return
  }

  const workspace = identify.caller?.workspace_ref
  if (!workspace) return
  const tree = await runJson<CmuxTree>($, "tree", "--workspace", workspaceId, "--json")
  await log($, { stage: "tree", callerPane, tree })
  let panes: CmuxPanes
  try {
    panes = await runJson<CmuxPanes>($, "list-panes", "--workspace", workspaceId, "--json")
  } catch (error) {
    await log($, { stage: "list-panes-error", error: String(error) })
    throw error
  }
  await log($, { stage: "list-panes", panes })
  const current = panes.panes?.find((pane) => pane.ref === callerPane)
  if (!current?.pixel_frame) {
    await log($, { stage: "missing-current-pane-geometry", callerPane })
    return
  }

  const treePanes = tree.windows
    ?.flatMap((window) => window.workspaces ?? [])
    .find((item) => item.ref === workspaceRef)?.panes ?? []
  const right = (panes.panes ?? [])
    .filter((pane) => {
      const frame = pane.pixel_frame
      const currentFrame = current.pixel_frame!
      return frame
        && frame.x > currentFrame.x
        && frame.y < currentFrame.y + currentFrame.height
        && frame.y + frame.height > currentFrame.y
    })
    .sort((left, right) => (left.pixel_frame?.x ?? 0) - (right.pixel_frame?.x ?? 0))
    .find((pane) => treePanes.some((candidate) => candidate.ref === pane.ref))
  if (!right) {
    await log($, { stage: "missing-right-navigator-pane", callerPane, paneRefs: panes.panes?.map((pane) => pane.ref), treePaneRefs: treePanes.map((pane) => pane.ref) })
    return
  }
  const rightTreePane = treePanes.find((pane) => pane.ref === right.ref)
  const rightSurface = rightTreePane?.surfaces?.[0]?.ref
  await log($, { stage: "right-pane", right: right.ref, rightSurface })

  const [worktree, repository] = await Promise.all([
    timed($, "git.existing-worktree", () => existingWorktree($, file), { file }),
    timed($, "git.repository-identity", () => repositoryIdentity($, file), { file }),
  ])
  const relativeFile = relativeTo(worktree, file)
  const sessionResult = await timed($, "git-navigator.ctl-sessions", () => $`git-navigator ctl sessions`.quiet())
  const sessions = sessionResult.text().trim().split("\n").filter(Boolean)
  await log($, { stage: "sessions", sessions })
  const processResult = await timed(
    $,
    "cmux.top",
    () => $`cmux top --workspace ${workspace} --processes --flat --format tsv`.quiet(),
    { workspace },
  )
  const navigatorProcess = processResult.text().split("\n").map((line) => line.split("\t"))
    .find((fields) => fields[3] === "process" && fields[5] === rightSurface && fields[6] === "git-navigator")
  const session = sessions.find((line) => {
    const fields = line.split("\t")
    return fields[3] === navigatorProcess?.[4]
  }) ?? sessions.find((line) => line.split("\t")[1] === worktree) ?? sessions[0]
  await log($, { stage: "selected-session", navigatorProcess, session })
  if (!session) {
    await log($, { stage: "missing-matching-session", directory, sessions })
    return
  }
  const socket = session?.split("\t")[2]
  if (!socket) {
    await log($, { stage: "missing-session-socket", session })
    return
  }

  if (state.socket !== socket) {
    state.socket = socket
    state.repositoryIdentity = undefined
    state.worktree = undefined
    state.file = undefined
    await log($, { stage: "navigator-session-changed", socket })
  }

  const commands: Array<readonly ["repository" | "worktree" | "file", string]> = []
  if (state.repositoryIdentity !== repository) commands.push(["repository", worktree])
  if (state.worktree !== worktree) commands.push(["worktree", worktree])
  if (state.file !== relativeFile) commands.push(["file", relativeFile])

  if (commands.length === 0) {
    await log($, {
      stage: "navigation-skipped",
      directory,
      file,
      eventCount,
      reason: "already-selected",
      durationMs: Math.round((performance.now() - started) * 100) / 100,
    })
    return
  }

  for (const [command, argument] of commands) {
    try {
      await timed(
        $,
        `git-navigator.ctl.${command}`,
        () => $`git-navigator ctl --socket ${socket} ${command} ${argument}`.quiet(),
        { argument, eventCount },
      )
      if (command === "repository") {
        state.repositoryIdentity = repository
        state.worktree = undefined
        state.file = undefined
      } else if (command === "worktree") {
        state.worktree = worktree
        state.file = undefined
      } else {
        state.file = relativeFile
      }
    } catch (error) {
      await log($, {
        stage: "navigation-command-failed",
        command,
        argument,
        eventCount,
        durationMs: Math.round((performance.now() - started) * 100) / 100,
        error: String(error),
      })
      throw error
    }
  }

  await log($, {
    stage: "navigation-complete",
    directory,
    file,
    eventCount,
    commandCount: commands.length,
    durationMs: Math.round((performance.now() - started) * 100) / 100,
  })
}

export const GitNavigatorRefresh: Plugin = async ({ $, directory }) => {
  let pending: PendingNavigation | undefined
  let timer: ReturnType<typeof setTimeout> | undefined
  let running = false
  const navigator: NavigatorState = {}

  const drain = async () => {
    if (running || !pending) return
    const next = pending
    pending = undefined
    running = true
    const started = performance.now()
    try {
      await refreshAdjacentNavigator($, directory, next.file, next.eventCount, navigator)
    } catch {
      await log($, {
        stage: "navigation-failed",
        file: next.file,
        eventCount: next.eventCount,
        durationMs: Math.round((performance.now() - started) * 100) / 100,
      })
      // Git Navigator and cmux are optional. File edits must not fail because
      // the companion panel is unavailable.
    } finally {
      running = false
      if (pending) {
        timer = setTimeout(() => {
          timer = undefined
          void drain()
        }, DEBOUNCE_MS)
      }
    }
  }

  return {
    event: async ({ event }) => {
      if (event.type !== "file.edited") return

      const file = editedPath(event as FileEditedEvent)
      if (!file) {
        await log($, { stage: "missing-edited-file", event })
        return
      }

      pending = {
        file,
        eventCount: (pending?.eventCount ?? 0) + 1,
      }
      await log($, { stage: "navigation-queued", directory, file, eventCount: pending.eventCount })
      if (timer) clearTimeout(timer)
      timer = setTimeout(() => {
        timer = undefined
        void drain()
      }, DEBOUNCE_MS)
    },
  }
}
