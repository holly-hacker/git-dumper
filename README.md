# git-dumper

This repository houses a tool to dump exposed .git repositories. This is a rewrite from the original [GitTools](https://github.com/internetwache/GitTools/)'s Dumper project, but in a real programming language with parallelism for massive speed gains (over [10x faster](https://asciinema.org/a/8Bz5jVhriCqxvNas87pjphHFN)).

## Why?
Many (lazy?) developers deploy their projects to their webservers through git: they run `git clone https://url/to/my-repo.git` in their web server's content directory. Doing this often leaves a `.git` folder exposed which an attacker can scrape and use to reconstruct your website's source code and version history. This tool does exactly that: scrape the .git directory so you can have a copy locally.

## Limitations
Git may run "garbage collection" on your repository which causes it to compact multiple object files into "pack" files. While object files can be found fairly easily through references from other object files, pack files dont seem to have explicit references to them and can not be downloaded without having a directory listing. If you do have a directory listing, you dont need this tool and can download the repository using `wget` :)

## Related projects
- [GitTools](https://github.com/internetwache/GitTools/), which inspired this project.
- [DotGit](https://github.com/davtur19/DotGit), a browser extension that automatically checks for exposed .git directories.
