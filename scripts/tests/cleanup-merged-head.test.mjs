import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import { cleanupMergedHead } from '../cleanup-merged-head.mjs';

const sha = 'a'.repeat(40);
const repository = { id: 7, full_name: 'owner/repo', default_branch: 'master' };
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

test('only eligible arm and reset jobs share the PR cancellation lock', () => {
  const workflow = readFileSync(new URL('../../.github/workflows/auto-merge.yml', import.meta.url), 'utf8');
  const resetStart = workflow.indexOf('\n  reset:');
  const lock = `    concurrency:\n      group: guarded-auto-merge-\${{ github.repository }}-\${{ github.event.issue.number || github.event.pull_request.number }}\n      cancel-in-progress: true`;
  assert.ok(resetStart > 0);
  assert.doesNotMatch(workflow, /^concurrency:/m);
  assert.ok(workflow.slice(0, resetStart).includes(lock));
  assert.ok(workflow.slice(resetStart).includes(lock));
  assert.equal((workflow.match(/^    concurrency:/gm) ?? []).length, 2);
});

function scenario(change = () => {}) {
  const current = structuredClone({ repository, number: 263, pr, statuses: [approval], comments: [comment],
    open: [], openBase: [], ref: { ref: 'refs/heads/agent/issue-263', object: { sha } } });
  change(current);
  const calls = [];
  const deletes = [];
  const waits = [];
  const request = async path => {
    calls.push(path);
    if (path.endsWith('/pulls/263')) return current.prSequence?.shift() ?? current.pr;
    if (path.includes('/statuses?')) return current.statuses;
    if (path.includes('/comments?')) return current.comments;
    if (path.includes('/pulls?') && path.includes('&base=')) return current.openBase;
    if (path.includes('/pulls?')) return current.open;
    if (path.includes('/git/ref/')) return current.ref;
    throw new Error(`unexpected API path: ${path}`);
  };
  return { current, calls, deletes, waits,
    run: () => cleanupMergedHead(current.repository, current.number, request, (branch, expected) => {
      deletes.push([branch, expected]);
    }, async ms => { waits.push(ms); }) };
}

test('deletes an already merged, authorized head with an explicit SHA lease', async () => {
  const s = scenario();
  assert.equal(await s.run(), true);
  assert.deepEqual(s.deletes, [['agent/issue-263', sha]]);
  assert.deepEqual(s.waits, []);
  assert.ok(s.calls.includes('repos/owner/repo/pulls?state=open&head=owner%3Aagent%2Fissue-263&per_page=1'));
  assert.ok(s.calls.includes('repos/owner/repo/pulls?state=open&base=agent%2Fissue-263&per_page=1'));
});

test('waits in the same run for a deferred native merge', async () => {
  const s = scenario(current => {
    current.prSequence = [{ ...current.pr, state: 'open', merged_at: null }, current.pr];
  });
  assert.equal(await s.run(), true);
  assert.deepEqual(s.waits, [15000]);
  assert.deepEqual(s.deletes, [['agent/issue-263', sha]]);
});

test('times out after 20 minutes of waits and leaves the branch untouched', async () => {
  const s = scenario(current => { current.pr.state = 'open'; current.pr.merged_at = null; });
  assert.equal(await s.run(), false);
  assert.equal(s.waits.length, 80);
  assert.ok(s.waits.every(ms => ms === 15000));
  assert.deepEqual(s.deletes, []);
});

for (const [name, change] of [
  ['unmerged closed PR', s => { s.pr.merged_at = null; }],
  ['human manual merge', s => { s.pr.merged_by.login = 'maintainer'; }],
  ['fork head', s => { s.pr.head.repo.id = 8; s.pr.head.repo.full_name = 'fork/repo'; }],
  ['moved branch', s => { s.ref.object.sha = 'b'.repeat(40); }],
  ['open PR sharing head', s => { s.open = [{ number: 264 }]; }],
  ['open PR based on branch', s => { s.openBase = [{ number: 264 }]; }],
  ['no workflow approval', s => { s.statuses = []; }],
  ['pending approval supersedes success', s => { s.statuses.unshift({ ...approval, state: 'pending' }); }],
  ['approval from another source', s => { s.statuses[0].description = 'manual status'; }],
  ['no exact auto-merge comment', s => { s.comments[0].body = '/auto-merge please'; }],
  ['invalid branch name', s => { s.pr.head.ref = '../unsafe'; }],
  ['invalid PR number', s => { s.number = 0; }],
]) {
  test(`skips ${name}`, async () => {
    const s = scenario(change);
    if (name === 'invalid PR number') await assert.rejects(s.run(), /invalid repository or PR number/);
    else assert.equal(await s.run(), false);
    assert.deepEqual(s.deletes, []);
  });
}
