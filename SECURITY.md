# Security reporting

Do not report suspected vulnerabilities, credentials, keys, tokens, customer
information, network identifiers, device identifiers, raw MQTT data, databases,
configuration, or sensitive screenshots in a GitHub issue or pull request.

This repository is currently a private development repository. Collaborators must
contact the repository owner through the private communication channel that was
used to grant repository access. Do not send the sensitive value itself unless the
owner provides an approved secure transfer method. If no such channel exists,
open a redacted issue that asks only for a private contact method and contains no
vulnerability details or sensitive values.

Before this repository is made public, the repository owner must enable GitHub
Private Vulnerability Reporting and replace this private-development instruction
with the verified reporting URL. Public release is blocked until that route exists
and has been tested.

If sensitive data was posted accidentally:

1. revoke or rotate the exposed credential immediately;
2. do not rely on editing or deleting the post to remove it from history;
3. contact the repository owner privately with the URL, not another copy of the
   secret;
4. preserve only redacted evidence needed to investigate the product behavior.

## 日本語

脆弱性の疑い、credential、鍵、token、顧客情報、network識別情報、device識別情報、
生MQTT data、DB、設定、機密を含むscreenshotをGitHub IssueやPull Requestへ書かないで
ください。

現在、このrepositoryは非公開の開発repositoryです。Collaboratorはrepository accessを
受け取ったときの非公開連絡経路でrepository ownerへ連絡してください。Ownerが安全な転送方法を
指定するまで、機密値そのものを送らないでください。その経路がない場合は、脆弱性の内容や
機密値を書かず、非公開の連絡方法だけを求める秘匿化済みIssueを作成してください。

このrepositoryを公開する前に、ownerはGitHub Private Vulnerability Reportingを有効化し、
この非公開開発向け案内を検証済みの報告URLへ置き換えます。報告経路を実際に確認するまで
公開releaseを行いません。

機密情報を誤って投稿した場合は、次の順序で対処します。

1. 公開したcredentialを直ちに失効またはrotationする。
2. 投稿の編集・削除だけで履歴から消えたと判断しない。
3. 秘密を再掲せず、投稿URLだけをrepository ownerへ非公開で伝える。
4. 製品動作の調査に必要な秘匿化済み証拠だけを残す。
