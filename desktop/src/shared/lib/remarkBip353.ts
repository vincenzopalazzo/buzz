/**
 * Keep BIP-353 payment names (`₿alice@example.com`) out of `mailto:` links.
 *
 * `remark-gfm`'s autolinker turns any `user@domain` into an email link while
 * parsing, so a human-readable Bitcoin payment name an agent writes after
 * paying (BIP-353 resolves it to a BOLT12 offer) would open a mail client.
 * The `₿` prefix is what distinguishes the two: when an autolinked address
 * directly follows `₿`, put it back as plain text. Explicit
 * `[label](mailto:…)` links are left alone.
 *
 * Runs right after `remark-gfm` (see `markdown/nodeCache.ts`).
 */

type MdNode = {
  type: string;
  value?: string;
  url?: string;
  children?: MdNode[];
};

const BITCOIN_SIGN = "₿";

function isBip353Autolink(prev: MdNode | undefined, node: MdNode): boolean {
  if (node.type !== "link" || !node.url?.startsWith("mailto:")) return false;
  if (prev?.type !== "text" || !prev.value?.endsWith(BITCOIN_SIGN))
    return false;
  const only = node.children?.length === 1 ? node.children[0] : undefined;
  return (
    only?.type === "text" && only.value === node.url.slice("mailto:".length)
  );
}

function unlinkBip353(node: MdNode): void {
  const children = node.children;
  if (!children) return;
  const out: MdNode[] = [];
  for (const child of children) {
    const prev = out[out.length - 1];
    if (prev && isBip353Autolink(prev, child)) {
      prev.value = `${prev.value}${child.children?.[0]?.value ?? ""}`;
      continue;
    }
    // Merge text that follows an unlinked name: "₿alice@x.com" + " now".
    if (
      prev?.type === "text" &&
      child.type === "text" &&
      prev.value?.includes(BITCOIN_SIGN)
    ) {
      prev.value = `${prev.value}${child.value ?? ""}`;
      continue;
    }
    unlinkBip353(child);
    out.push(child);
  }
  node.children = out;
}

export default function remarkBip353() {
  return (tree: MdNode) => {
    unlinkBip353(tree);
  };
}
