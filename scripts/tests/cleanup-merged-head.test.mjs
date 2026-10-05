import assert from 'node:assert/strict';
import test from 'node:test';
import { cleanupMergedHead } from '../cleanup-merged-head.mjs';

const sha = 'a'.repeat(40);
const event = {
  action: 'closed',
  repository: { id: 7, full_name: 'owner/repo', default_branch: 'master' },
  pull_request: { number: 263, merged: true, head: { sha } },
};
const pr = {
  number: 263, state: 'closed', merged_at: '2026-10-05T01:00:00Z',
  merged_by: { login: 'github-actions[bot]' },
  base: { ref: 'master' },
  head: { ref: 'agent/issue-263', sha, repo: { id: 7, full_name: 'owner/repo' } },
};
const approval = {
  context: 'human approval', state: 'success',
  description: 'Authorized /auto-merge comment',
  creator: { login: 'github-actions[bot]' }, created_at: '2026-10-05T00:30:00Z',
};
const comment = {
  body: '/auto-merge', user: { type: 'User' },
  author_association: 'MEMBER', created_at: '2026-10-05T00:29:00Z',
};

function scenario(change = () => {}) {
  const current = structuredClone({ event, pr, statuses: [approval], comments: [comment],
    open: [], openBase: [], ref: { ref: 'refs/heads/agent/issue-263', object: { sha } } });
  change(current);
  const calls = [];
  const deletes = [];
  const request = async path => {
    calls.push(path);
    if (path.endsWith('/pulls/263')) return current.pr;
    if (path.includes('/statuses?')) return current.statuses;
    if (path.includes('/comments?')) return current.comments;
    if (path.includes('/pulls?') && path.includes('&base=')) return current.openBase;
    if (path.includes('/pulls?')) return current.open;
    if (path.includes('/git/ref/')) return current.ref;
    throw new Error(`unexpected API path: ${path}`);
  };
  return { current, calls, deletes,
    run: () => cleanupMergedHead(current.event, request, (branch, expected) => {
      deletes.push([branch, expected]);
    }) };
}

test('deletes only the matching head with an explicit expected SHA', async () => {
  const s = scenario();
  assert.equal(await s.run(), true);
  assert.deepEqual(s.deletes, [['agent/issue-263', sha]]);
  assert.ok(s.calls.includes('repos/owner/repo/pulls?state=open&head=owner%3Aagent%2Fissue-263&per_page=1'));
  assert.ok(s.calls.includes('repos/owner/repo/pulls?state=open&base=agent%2Fissue-263&per_page=1'));
});

for (const [name, change] of [
  ['unmerged', s => { s.event.pull_request.merged = false; }],
  ['human manual merge', s => { s.pr.merged_by.login = 'maintainer'; }],
  ['fork head', s => { s.pr.head.repo.id = 8; s.pr.head.repo.full_name = 'fork/repo'; }],
  ['moved branch', s => { s.ref.object.sha = 'b'.repeat(40); }],
  ['open PR sharing branch', s => { s.open = [{ number: 264 }]; }],
  ['open PR based on branch', s => { s.openBase = [{ number: 264 }]; }],
  ['no workflow approval', s => { s.statuses = []; }],
  ['pending approval supersedes success', s => { s.statuses.unshift({ ...approval, state: 'pending' }); }],
  ['approval from another source', s => { s.statuses[0].description = 'manual status'; }],
  ['no exact auto-merge comment', s => { s.comments[0].body = '/auto-merge please'; }],
  ['invalid branch name', s => { s.pr.head.ref = '../unsafe'; }],
]) {
  test(`skips ${name}`, async () => {
    const s = scenario(change);
    assert.equal(await s.run(), false);
    assert.deepEqual(s.deletes, []);
  });
}
