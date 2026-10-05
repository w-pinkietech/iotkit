import { execFileSync } from 'node:child_process';
import { readFileSync } from 'node:fs';
import { pathToFileURL } from 'node:url';

const approvalDescription = 'Authorized /auto-merge comment';
const shaPattern = /^[0-9a-f]{40}$/;

// The caller supplies only trusted GitHub metadata. Ref checks and the lease
// keep an updated or reused branch intact even if it moves after the API read.
export async function cleanupMergedHead(event, request, deleteBranch) {
  if (event.action !== 'closed' || event.pull_request?.merged !== true) return false;

  const repo = event.repository?.full_name;
  const number = event.pull_request.number;
  if (!/^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/.test(repo) ||
      !Number.isSafeInteger(number) || number < 1) throw new Error('invalid event repository or PR number');

  const pr = await request(`repos/${repo}/pulls/${number}`);
  const branch = pr.head?.ref;
  const sha = pr.head?.sha;
  if (pr.number !== number || pr.state !== 'closed' || !pr.merged_at ||
      pr.merged_by?.login !== 'github-actions[bot]' ||
      pr.base?.ref !== event.repository.default_branch ||
      pr.head?.repo?.full_name !== repo ||
      pr.head.repo.id !== event.repository.id ||
      !shaPattern.test(sha) || sha !== event.pull_request.head.sha ||
      typeof branch !== 'string' || branch === event.repository.default_branch) return false;
  try {
    execFileSync('git', ['check-ref-format', `refs/heads/${branch}`], { stdio: 'ignore' });
  } catch {
    return false;
  }

  // GitHub returns statuses newest first. Only the latest approval state counts.
  let approval;
  for (let page = 1; !approval; page++) {
    const statuses = await request(`repos/${repo}/commits/${sha}/statuses?per_page=100&page=${page}`);
    approval = statuses.find(status => status.context === 'human approval');
    if (approval || statuses.length < 100) break;
  }
  if (approval?.state !== 'success' || approval.description !== approvalDescription ||
      approval.creator?.login !== 'github-actions[bot]') return false;

  // Require the exact authorized command as well as this workflow's status.
  let approvedComment = false;
  for (let page = 1; !approvedComment; page++) {
    const comments = await request(`repos/${repo}/issues/${number}/comments?per_page=100&page=${page}`);
    approvedComment = comments.some(comment =>
      comment.body === '/auto-merge' && comment.user?.type === 'User' &&
      ['OWNER', 'MEMBER', 'COLLABORATOR'].includes(comment.author_association) &&
      Date.parse(comment.created_at) <= Date.parse(approval.created_at));
    if (approvedComment || comments.length < 100) break;
  }
  if (!approvedComment) return false;

  const owner = repo.split('/')[0];
  const open = await request(`repos/${repo}/pulls?state=open&head=${encodeURIComponent(`${owner}:${branch}`)}&per_page=1`);
  if (open.length) return false;
  const basedOnBranch = await request(`repos/${repo}/pulls?state=open&base=${encodeURIComponent(branch)}&per_page=1`);
  if (basedOnBranch.length) return false;

  const refPath = branch.split('/').map(encodeURIComponent).join('/');
  let ref;
  try {
    ref = await request(`repos/${repo}/git/ref/heads/${refPath}`);
  } catch (error) {
    if (error.status === 404) return false;
    throw error;
  }
  if (ref.object?.sha !== sha || ref.ref !== `refs/heads/${branch}`) return false;

  await deleteBranch(branch, sha);
  return true;
}

async function request(path) {
  const response = await fetch(`https://api.github.com/${path}`, {
    headers: {
      authorization: `Bearer ${process.env.GITHUB_TOKEN}`,
      accept: 'application/vnd.github+json',
      'x-github-api-version': '2022-11-28',
    },
  });
  if (!response.ok) throw Object.assign(new Error(`GitHub API returned ${response.status} for ${path}`), { status: response.status });
  return response.json();
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const event = JSON.parse(readFileSync(process.env.GITHUB_EVENT_PATH, 'utf8'));
  const deleted = await cleanupMergedHead(event, request, (branch, sha) => {
    execFileSync('git', ['push', `--force-with-lease=refs/heads/${branch}:${sha}`,
      'origin', `:refs/heads/${branch}`], { stdio: 'inherit' });
  });
  console.log(deleted ? 'Deleted approved merged PR head' : 'Head branch is ineligible or changed; skipped');
}
