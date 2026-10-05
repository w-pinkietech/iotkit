import { execFileSync } from 'node:child_process';
import { readFileSync } from 'node:fs';
import { pathToFileURL } from 'node:url';

const approvalDescription = 'Authorized /auto-merge comment';
const shaPattern = /^[0-9a-f]{40}$/;

// Called only after the authorized /auto-merge step successfully arms native
// auto-merge. The wait stays in that same workflow run; timeout means skip.
export async function cleanupMergedHead(repository, number, request, deleteBranch,
  wait = ms => new Promise(resolve => setTimeout(resolve, ms)),
  linkedIssues = closingIssuesReferences, closeIssue = closeIssueOnGitHub,
  approvedIssueRefs = []) {
  if (!/^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/.test(repository?.full_name) ||
      !Number.isSafeInteger(number) || number < 1) throw new Error('invalid repository or PR number');
  const repo = repository.full_name;
  let pr;
  // 80 waits of 15 seconds = at most 20 minutes for deferred native merge.
  for (let attempt = 0; attempt <= 80; attempt++) {
    pr = await request(`repos/${repo}/pulls/${number}`);
    if (pr.merged_at || pr.state !== 'open') break;
    if (attempt < 80) await wait(15000);
  }

  const sha = pr.head?.sha;
  if (pr.number !== number || pr.state !== 'closed' || !pr.merged_at ||
      pr.merged_by?.login !== 'github-actions[bot]' ||
      pr.base?.ref !== repository.default_branch ||
      !shaPattern.test(sha)) return false;

  // GitHub returns statuses newest first. Only the latest approval state counts.
  let approval;
  for (let page = 1; !approval; page++) {
    const statuses = await request(`repos/${repo}/commits/${sha}/statuses?per_page=100&page=${page}`);
    approval = statuses.find(status => status.context === 'human approval');
    if (approval || statuses.length < 100) break;
  }
  if (approval?.state !== 'success' || approval.description !== approvalDescription ||
      approval.creator?.login !== 'github-actions[bot]') return false;

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

  // Issue closure and branch deletion have separate eligibility and errors.
  // A missing/fork head must not stop an authorized issue closure.
  let closedIssues = 0;
  const errors = [];
  try {
    const linked = await linkedIssues(repo, number);
    const approved = sameRepoIssueNumbers(approvedIssueRefs, repo);
    const currentRefs = sameRepoIssueNumbers(linked, repo);
    if (JSON.stringify(approved) !== JSON.stringify(currentRefs)) {
      console.log('Recognized issue references changed after approval; skipping issue closure');
    } else {
      for (const issueNumber of currentRefs) {
        try {
          const current = await request(`repos/${repo}/issues/${issueNumber}`);
          if (current.state !== 'open') continue;
          await closeIssue(repo, issueNumber);
          closedIssues++;
        } catch (error) {
          errors.push(error);
        }
      }
    }
  } catch (error) {
    errors.push(error);
  }

  let deletedBranch = false;
  let branchError;
  try {
    deletedBranch = await deleteEligibleBranch(repository, pr, request, deleteBranch);
  } catch (error) {
    branchError = error;
  }
  if (branchError) errors.push(branchError);
  if (errors.length > 1) throw new AggregateError(errors, 'approved PR cleanup had multiple failures');
  if (errors.length === 1) throw errors[0];
  return deletedBranch || closedIssues > 0;
}

function sameRepoIssueNumbers(refs, repo) {
  if (!Array.isArray(refs)) throw new Error('invalid recognized issue references');
  const [owner, name] = repo.toLowerCase().split('/');
  const numbers = new Set();
  for (const issue of refs) {
    if (issue.repository?.owner?.login?.toLowerCase() !== owner ||
        issue.repository?.name?.toLowerCase() !== name) continue;
    if (!Number.isSafeInteger(issue.number) || issue.number < 1) throw new Error('invalid linked issue number');
    numbers.add(issue.number);
  }
  return [...numbers].sort((a, b) => a - b);
}

async function deleteEligibleBranch(repository, pr, request, deleteBranch) {
  const repo = repository.full_name;
  const branch = pr.head?.ref;
  const sha = pr.head?.sha;
  if (pr.head?.repo?.full_name !== repo || pr.head.repo.id !== repository.id ||
      typeof branch !== 'string' || branch === repository.default_branch) return false;
  try {
    execFileSync('git', ['check-ref-format', `refs/heads/${branch}`], { stdio: 'ignore' });
  } catch {
    return false;
  }

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

  // The explicit lease atomically rejects deletion if the remote SHA moves.
  await deleteBranch(branch, sha);
  return true;
}

function closingIssuesReferences(repo, number) {
  const output = execFileSync('gh', ['pr', 'view', String(number), '--repo', repo,
    '--json', 'closingIssuesReferences'], { encoding: 'utf8' });
  return JSON.parse(output).closingIssuesReferences;
}

async function closeIssueOnGitHub(repo, number) {
  const response = await fetch(`https://api.github.com/repos/${repo}/issues/${number}`, {
    method: 'PATCH',
    headers: {
      authorization: `Bearer ${process.env.GITHUB_TOKEN}`,
      accept: 'application/vnd.github+json',
      'content-type': 'application/json',
      'x-github-api-version': '2022-11-28',
    },
    body: JSON.stringify({ state: 'closed' }),
  });
  if (!response.ok) throw new Error(`GitHub API returned ${response.status} while closing issue ${number}`);
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
  const number = Number(process.env.PR_NUMBER);
  let approvedIssueRefs;
  try {
    approvedIssueRefs = JSON.parse(Buffer.from(process.env.APPROVED_ISSUE_REFS, 'base64').toString('utf8')).closingIssuesReferences;
  } catch {
    approvedIssueRefs = null; // Fail issue closure while still checking branch eligibility.
  }
  const changed = await cleanupMergedHead(event.repository, number, request, (branch, sha) => {
    execFileSync('git', ['push', `--force-with-lease=refs/heads/${branch}:${sha}`,
      'origin', `:refs/heads/${branch}`], { stdio: 'inherit' });
  }, undefined, undefined, undefined, approvedIssueRefs);
  console.log(changed ? 'Processed approved merged PR cleanup' : 'No eligible cleanup action');
}
