# armorcd

`armor` is a secure continuous delivery system for Linux packages.

```
armor login
armor logout
armor whoami

armor domains list
armor domains add example.com
armor domains rm example.com

armor repo create deb uri distribution component1 component2 ...
armor repo check uri
armor repo edit uri distribution key=value

armor pkgs list
armor pkgs add --repository uri --dist distname --component compname ./package.deb
armor pkgs rm foo-1.0
```
