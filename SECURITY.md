# Security reporting

If you believe you have found a vulnerability in IoTKit, use GitHub's
[private vulnerability reporting form](https://github.com/w-pinkietech/iotkit/security/advisories/new).
GitHub explains this process in its
[private vulnerability reporting guidance](https://docs.github.com/code-security/security-advisories/guidance-on-reporting-and-writing/privately-reporting-a-security-vulnerability/).

Do not report suspected vulnerabilities, credentials, keys, tokens, customer
information, network identifiers, device identifiers, raw MQTT data, databases,
configuration, or sensitive screenshots in a GitHub Issue or Pull Request. Do
not send sensitive values through an unrelated public or private channel.

If sensitive data was posted accidentally:

1. revoke or rotate the exposed credential immediately;
2. do not rely on editing or deleting the post to remove it from history;
3. contact the repository maintainer privately with the URL, without copying the
   secret or vulnerability details again;
4. preserve only redacted evidence needed to investigate the product behavior.

## 日本語

IoTKitの脆弱性を発見した可能性がある場合は、GitHubの
[非公開脆弱性報告フォーム](https://github.com/w-pinkietech/iotkit/security/advisories/new)
を使用してください。この仕組みについては、GitHubの
[非公開脆弱性報告ガイド](https://docs.github.com/code-security/security-advisories/guidance-on-reporting-and-writing/privately-reporting-a-security-vulnerability/)
も参照できます。

脆弱性の疑い、credential、鍵、token、顧客情報、network識別情報、device識別情報、
生MQTT data、DB、設定、機密を含むscreenshotをGitHub IssueやPull Requestへ書かないで
ください。無関係な公開・非公開の連絡経路で機密値を送らないでください。

機密情報を誤って投稿した場合は、次の順序で対処します。

1. 公開したcredentialを直ちに失効またはrotationする。
2. 投稿の編集・削除だけで履歴から消えたと判断しない。
3. 秘密や脆弱性の詳細を再掲せず、投稿URLだけをrepository maintainerへ非公開で伝える。
4. 製品動作の調査に必要な秘匿化済み証拠だけを残す。
