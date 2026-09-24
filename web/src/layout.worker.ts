import { compactSlots } from "./positions";

type Request = { token: number; root: string; members: string[]; edges: [string, string][] };

self.onmessage = (event: MessageEvent<Request>) => {
  const { token, root, members, edges } = event.data;
  const adjacency = new Map([root, ...members].map(id => [id, new Set<string>()]));
  for (const [a, b] of edges) {
    adjacency.get(a)?.add(b);
    adjacency.get(b)?.add(a);
  }
  const slots = compactSlots(root, members, adjacency, 1000, 100000);
  self.postMessage({ token, slots: slots ? [...slots] : null });
};
