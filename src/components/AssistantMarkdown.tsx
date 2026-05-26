import { Children, isValidElement, useMemo, useState, type ReactElement, type ReactNode } from "react";
import { Check, Copy } from "lucide-react";
import ReactMarkdown, { type Components } from "react-markdown";
import remarkGfm from "remark-gfm";
import { openUrl } from "@tauri-apps/plugin-opener";
import { ChatInlineImage } from "./ChatInlineImage";
import { extractImageRefs } from "../lib/chatImageDetect";

type MarkdownNode = {
  type?: string;
  value?: string;
  children?: MarkdownNode[];
  url?: string;
  alt?: string;
  title?: string | null;
};

const IMAGE_PATH_RE = /\.(?:png|jpe?g|webp|gif|bmp|tiff?|avif)(?:[?#].*)?$/i;
const CODE_LANG_RE = /language-([\w-]+)/;
const SKIP_PLAIN_IMAGE_SCAN = new Set(["code", "inlineCode", "link", "image", "html"]);
const MARKDOWN_PLUGINS = [remarkGfm, remarkPlainImageRefs];

export function AssistantMarkdown({ text, seenPaths }: { text: string; seenPaths?: Set<string> }) {
  const components = useMemo<Components>(
    () => ({
      a({ href, children }) {
        if (href && isImagePath(href)) {
          return <MarkdownImage path={href} seenPaths={seenPaths} />;
        }

        if (!href || !isExternalUrl(href)) return <span>{children}</span>;

        return (
          <a
            href={href}
            onClick={(e) => {
              e.preventDefault();
              void openUrl(href).catch(() => {});
            }}
          >
            {children}
          </a>
        );
      },
      code({ className, children }) {
        return <code className={className}>{children}</code>;
      },
      img({ src }) {
        return src ? <MarkdownImage path={src} seenPaths={seenPaths} /> : null;
      },
      pre({ children }) {
        return <CodeBlock>{children}</CodeBlock>;
      },
      table({ children }) {
        return (
          <div className="cm-md__tablewrap">
            <table>{children}</table>
          </div>
        );
      },
    }),
    [seenPaths],
  );

  return (
    <div className="cm-md">
      <ReactMarkdown
        remarkPlugins={MARKDOWN_PLUGINS}
        components={components}
        skipHtml
      >
        {text}
      </ReactMarkdown>
    </div>
  );
}

function MarkdownImage({ path, seenPaths }: { path: string; seenPaths?: Set<string> }) {
  if (seenPaths?.has(path)) return <span className="cm-md__image-path">{path}</span>;
  seenPaths?.add(path);
  return <ChatInlineImage path={path} />;
}

function CodeBlock({ children }: { children: ReactNode }) {
  const [copied, setCopied] = useState(false);
  const child = Children.toArray(children)[0];
  const className = isCodeElement(child) ? child.props.className : undefined;
  const language = CODE_LANG_RE.exec(className ?? "")?.[1] ?? "text";
  const code = textFromNode(children).replace(/\n$/, "");

  const copyCode = async () => {
    try {
      await navigator.clipboard.writeText(code);
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1100);
    } catch {
      /* Best-effort clipboard affordance. */
    }
  };

  return (
    <div className="cm-codeblock">
      <div className="cm-codeblock__head">
        <span className="cm-codeblock__lang">{language}</span>
        <button
          className="cm-codeblock__copy"
          type="button"
          aria-label="Copy code"
          title="Copy code"
          onClick={() => void copyCode()}
        >
          {copied ? <Check size={13} /> : <Copy size={13} />}
        </button>
      </div>
      <pre>{children}</pre>
    </div>
  );
}

function isCodeElement(node: ReactNode): node is ReactElement<{ className?: string; children?: ReactNode }> {
  return isValidElement<{ className?: string; children?: ReactNode }>(node) && node.type === "code";
}

function textFromNode(node: ReactNode): string {
  if (typeof node === "string" || typeof node === "number") return String(node);
  if (Array.isArray(node)) return node.map(textFromNode).join("");
  if (isValidElement<{ children?: ReactNode }>(node)) return textFromNode(node.props.children);
  return "";
}

function isImagePath(value: string): boolean {
  return IMAGE_PATH_RE.test(value);
}

function isExternalUrl(value: string): boolean {
  return /^(?:https?:|mailto:)/i.test(value);
}

function remarkPlainImageRefs() {
  return (tree: MarkdownNode) => {
    transformPlainImageRefs(tree);
  };
}

function transformPlainImageRefs(node: MarkdownNode) {
  if (!node.children) return;

  const next: MarkdownNode[] = [];
  for (const child of node.children) {
    if (child.type === "inlineCode" && typeof child.value === "string") {
      const path = exactPlainImagePath(child.value);
      next.push(path ? { type: "image", url: path, alt: "", title: null } : child);
      continue;
    }

    if (child.type === "text" && typeof child.value === "string") {
      next.push(...splitPlainImageText(child.value));
      continue;
    }

    if (!SKIP_PLAIN_IMAGE_SCAN.has(child.type ?? "")) {
      transformPlainImageRefs(child);
    }
    next.push(child);
  }

  node.children = next;
}

function splitPlainImageText(value: string): MarkdownNode[] {
  const refs = extractImageRefs(value).filter((r) => r.kind === "plain");
  if (refs.length === 0) return [{ type: "text", value }];

  const nodes: MarkdownNode[] = [];
  let cursor = 0;
  for (const ref of refs) {
    if (ref.start > cursor) nodes.push({ type: "text", value: value.slice(cursor, ref.start) });
    nodes.push({ type: "image", url: ref.path, alt: "", title: null });
    cursor = ref.end;
  }
  if (cursor < value.length) nodes.push({ type: "text", value: value.slice(cursor) });
  return nodes;
}

function exactPlainImagePath(value: string): string | null {
  const trimmed = value.trim();
  if (!trimmed) return null;

  const refs = extractImageRefs(trimmed).filter((r) => r.kind === "plain");
  if (refs.length !== 1) return null;

  const [ref] = refs;
  return ref.start === 0 && ref.end === trimmed.length ? ref.path : null;
}
