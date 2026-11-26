_geectl() {
    local i cur prev opts cmd
    COMPREPLY=()
    if [[ "${BASH_VERSINFO[0]}" -ge 4 ]]; then
        cur="$2"
    else
        cur="${COMP_WORDS[COMP_CWORD]}"
    fi
    prev="$3"
    cmd=""
    opts=""

    for i in "${COMP_WORDS[@]:0:COMP_CWORD}"
    do
        case "${cmd},${i}" in
            ",$1")
                cmd="geectl"
                ;;
            geectl,add)
                cmd="geectl__subcmd__add"
                ;;
            geectl,addremove)
                cmd="geectl__subcmd__addremove"
                ;;
            geectl,branch)
                cmd="geectl__subcmd__branch"
                ;;
            geectl,build)
                cmd="geectl__subcmd__build"
                ;;
            geectl,changelog)
                cmd="geectl__subcmd__changelog"
                ;;
            geectl,checkout)
                cmd="geectl__subcmd__checkout"
                ;;
            geectl,commit)
                cmd="geectl__subcmd__commit"
                ;;
            geectl,config)
                cmd="geectl__subcmd__config"
                ;;
            geectl,diff)
                cmd="geectl__subcmd__diff"
                ;;
            geectl,gitea)
                cmd="geectl__subcmd__gitea"
                ;;
            geectl,help)
                cmd="geectl__subcmd__help"
                ;;
            geectl,local-build)
                cmd="geectl__subcmd__local__subcmd__build"
                ;;
            geectl,maintenance)
                cmd="geectl__subcmd__maintenance"
                ;;
            geectl,meta)
                cmd="geectl__subcmd__meta"
                ;;
            geectl,person)
                cmd="geectl__subcmd__person"
                ;;
            geectl,prjconf)
                cmd="geectl__subcmd__prjconf"
                ;;
            geectl,pull)
                cmd="geectl__subcmd__pull"
                ;;
            geectl,pull-request)
                cmd="geectl__subcmd__pull__subcmd__request"
                ;;
            geectl,remote)
                cmd="geectl__subcmd__remote"
                ;;
            geectl,request)
                cmd="geectl__subcmd__request"
                ;;
            geectl,resolve)
                cmd="geectl__subcmd__resolve"
                ;;
            geectl,rm)
                cmd="geectl__subcmd__rm"
                ;;
            geectl,service)
                cmd="geectl__subcmd__service"
                ;;
            geectl,status)
                cmd="geectl__subcmd__status"
                ;;
            geectl,update)
                cmd="geectl__subcmd__update"
                ;;
            geectl,version)
                cmd="geectl__subcmd__version"
                ;;
            geectl__subcmd__branch,branch-package)
                cmd="geectl__subcmd__branch__subcmd__branch__subcmd__package"
                ;;
            geectl__subcmd__branch,copy-package)
                cmd="geectl__subcmd__branch__subcmd__copy__subcmd__package"
                ;;
            geectl__subcmd__branch,copy-project)
                cmd="geectl__subcmd__branch__subcmd__copy__subcmd__project"
                ;;
            geectl__subcmd__branch,create)
                cmd="geectl__subcmd__branch__subcmd__create"
                ;;
            geectl__subcmd__branch,detach-branch)
                cmd="geectl__subcmd__branch__subcmd__detach__subcmd__branch"
                ;;
            geectl__subcmd__branch,help)
                cmd="geectl__subcmd__branch__subcmd__help"
                ;;
            geectl__subcmd__branch,link-package)
                cmd="geectl__subcmd__branch__subcmd__link__subcmd__package"
                ;;
            geectl__subcmd__branch,link-to-branch)
                cmd="geectl__subcmd__branch__subcmd__link__subcmd__to__subcmd__branch"
                ;;
            geectl__subcmd__branch,repair-link)
                cmd="geectl__subcmd__branch__subcmd__repair__subcmd__link"
                ;;
            geectl__subcmd__branch,set-devel-project)
                cmd="geectl__subcmd__branch__subcmd__set__subcmd__devel__subcmd__project"
                ;;
            geectl__subcmd__branch,set-link-rev)
                cmd="geectl__subcmd__branch__subcmd__set__subcmd__link__subcmd__rev"
                ;;
            geectl__subcmd__branch,set-link-rev-project)
                cmd="geectl__subcmd__branch__subcmd__set__subcmd__link__subcmd__rev__subcmd__project"
                ;;
            geectl__subcmd__branch__subcmd__help,branch-package)
                cmd="geectl__subcmd__branch__subcmd__help__subcmd__branch__subcmd__package"
                ;;
            geectl__subcmd__branch__subcmd__help,copy-package)
                cmd="geectl__subcmd__branch__subcmd__help__subcmd__copy__subcmd__package"
                ;;
            geectl__subcmd__branch__subcmd__help,copy-project)
                cmd="geectl__subcmd__branch__subcmd__help__subcmd__copy__subcmd__project"
                ;;
            geectl__subcmd__branch__subcmd__help,create)
                cmd="geectl__subcmd__branch__subcmd__help__subcmd__create"
                ;;
            geectl__subcmd__branch__subcmd__help,detach-branch)
                cmd="geectl__subcmd__branch__subcmd__help__subcmd__detach__subcmd__branch"
                ;;
            geectl__subcmd__branch__subcmd__help,help)
                cmd="geectl__subcmd__branch__subcmd__help__subcmd__help"
                ;;
            geectl__subcmd__branch__subcmd__help,link-package)
                cmd="geectl__subcmd__branch__subcmd__help__subcmd__link__subcmd__package"
                ;;
            geectl__subcmd__branch__subcmd__help,link-to-branch)
                cmd="geectl__subcmd__branch__subcmd__help__subcmd__link__subcmd__to__subcmd__branch"
                ;;
            geectl__subcmd__branch__subcmd__help,repair-link)
                cmd="geectl__subcmd__branch__subcmd__help__subcmd__repair__subcmd__link"
                ;;
            geectl__subcmd__branch__subcmd__help,set-devel-project)
                cmd="geectl__subcmd__branch__subcmd__help__subcmd__set__subcmd__devel__subcmd__project"
                ;;
            geectl__subcmd__branch__subcmd__help,set-link-rev)
                cmd="geectl__subcmd__branch__subcmd__help__subcmd__set__subcmd__link__subcmd__rev"
                ;;
            geectl__subcmd__branch__subcmd__help,set-link-rev-project)
                cmd="geectl__subcmd__branch__subcmd__help__subcmd__set__subcmd__link__subcmd__rev__subcmd__project"
                ;;
            geectl__subcmd__build,abort-build)
                cmd="geectl__subcmd__build__subcmd__abort__subcmd__build"
                ;;
            geectl__subcmd__build,build-history)
                cmd="geectl__subcmd__build__subcmd__build__subcmd__history"
                ;;
            geectl__subcmd__build,build-log)
                cmd="geectl__subcmd__build__subcmd__build__subcmd__log"
                ;;
            geectl__subcmd__build,build-status)
                cmd="geectl__subcmd__build__subcmd__build__subcmd__status"
                ;;
            geectl__subcmd__build,buildconfig)
                cmd="geectl__subcmd__build__subcmd__buildconfig"
                ;;
            geectl__subcmd__build,buildinfo)
                cmd="geectl__subcmd__build__subcmd__buildinfo"
                ;;
            geectl__subcmd__build,check-constraints)
                cmd="geectl__subcmd__build__subcmd__check__subcmd__constraints"
                ;;
            geectl__subcmd__build,dependson)
                cmd="geectl__subcmd__build__subcmd__dependson"
                ;;
            geectl__subcmd__build,download-binary)
                cmd="geectl__subcmd__build__subcmd__download__subcmd__binary"
                ;;
            geectl__subcmd__build,fileinfo)
                cmd="geectl__subcmd__build__subcmd__fileinfo"
                ;;
            geectl__subcmd__build,help)
                cmd="geectl__subcmd__build__subcmd__help"
                ;;
            geectl__subcmd__build,job-history)
                cmd="geectl__subcmd__build__subcmd__job__subcmd__history"
                ;;
            geectl__subcmd__build,job-status)
                cmd="geectl__subcmd__build__subcmd__job__subcmd__status"
                ;;
            geectl__subcmd__build,list-binaries)
                cmd="geectl__subcmd__build__subcmd__list__subcmd__binaries"
                ;;
            geectl__subcmd__build,list-repositories)
                cmd="geectl__subcmd__build__subcmd__list__subcmd__repositories"
                ;;
            geectl__subcmd__build,rebuild-package)
                cmd="geectl__subcmd__build__subcmd__rebuild__subcmd__package"
                ;;
            geectl__subcmd__build,restart-build)
                cmd="geectl__subcmd__build__subcmd__restart__subcmd__build"
                ;;
            geectl__subcmd__build,results)
                cmd="geectl__subcmd__build__subcmd__results"
                ;;
            geectl__subcmd__build,wait-for-results)
                cmd="geectl__subcmd__build__subcmd__wait__subcmd__for__subcmd__results"
                ;;
            geectl__subcmd__build,whatdependson)
                cmd="geectl__subcmd__build__subcmd__whatdependson"
                ;;
            geectl__subcmd__build,wipe-binaries)
                cmd="geectl__subcmd__build__subcmd__wipe__subcmd__binaries"
                ;;
            geectl__subcmd__build__subcmd__help,abort-build)
                cmd="geectl__subcmd__build__subcmd__help__subcmd__abort__subcmd__build"
                ;;
            geectl__subcmd__build__subcmd__help,build-history)
                cmd="geectl__subcmd__build__subcmd__help__subcmd__build__subcmd__history"
                ;;
            geectl__subcmd__build__subcmd__help,build-log)
                cmd="geectl__subcmd__build__subcmd__help__subcmd__build__subcmd__log"
                ;;
            geectl__subcmd__build__subcmd__help,build-status)
                cmd="geectl__subcmd__build__subcmd__help__subcmd__build__subcmd__status"
                ;;
            geectl__subcmd__build__subcmd__help,buildconfig)
                cmd="geectl__subcmd__build__subcmd__help__subcmd__buildconfig"
                ;;
            geectl__subcmd__build__subcmd__help,buildinfo)
                cmd="geectl__subcmd__build__subcmd__help__subcmd__buildinfo"
                ;;
            geectl__subcmd__build__subcmd__help,check-constraints)
                cmd="geectl__subcmd__build__subcmd__help__subcmd__check__subcmd__constraints"
                ;;
            geectl__subcmd__build__subcmd__help,dependson)
                cmd="geectl__subcmd__build__subcmd__help__subcmd__dependson"
                ;;
            geectl__subcmd__build__subcmd__help,download-binary)
                cmd="geectl__subcmd__build__subcmd__help__subcmd__download__subcmd__binary"
                ;;
            geectl__subcmd__build__subcmd__help,fileinfo)
                cmd="geectl__subcmd__build__subcmd__help__subcmd__fileinfo"
                ;;
            geectl__subcmd__build__subcmd__help,help)
                cmd="geectl__subcmd__build__subcmd__help__subcmd__help"
                ;;
            geectl__subcmd__build__subcmd__help,job-history)
                cmd="geectl__subcmd__build__subcmd__help__subcmd__job__subcmd__history"
                ;;
            geectl__subcmd__build__subcmd__help,job-status)
                cmd="geectl__subcmd__build__subcmd__help__subcmd__job__subcmd__status"
                ;;
            geectl__subcmd__build__subcmd__help,list-binaries)
                cmd="geectl__subcmd__build__subcmd__help__subcmd__list__subcmd__binaries"
                ;;
            geectl__subcmd__build__subcmd__help,list-repositories)
                cmd="geectl__subcmd__build__subcmd__help__subcmd__list__subcmd__repositories"
                ;;
            geectl__subcmd__build__subcmd__help,rebuild-package)
                cmd="geectl__subcmd__build__subcmd__help__subcmd__rebuild__subcmd__package"
                ;;
            geectl__subcmd__build__subcmd__help,restart-build)
                cmd="geectl__subcmd__build__subcmd__help__subcmd__restart__subcmd__build"
                ;;
            geectl__subcmd__build__subcmd__help,results)
                cmd="geectl__subcmd__build__subcmd__help__subcmd__results"
                ;;
            geectl__subcmd__build__subcmd__help,wait-for-results)
                cmd="geectl__subcmd__build__subcmd__help__subcmd__wait__subcmd__for__subcmd__results"
                ;;
            geectl__subcmd__build__subcmd__help,whatdependson)
                cmd="geectl__subcmd__build__subcmd__help__subcmd__whatdependson"
                ;;
            geectl__subcmd__build__subcmd__help,wipe-binaries)
                cmd="geectl__subcmd__build__subcmd__help__subcmd__wipe__subcmd__binaries"
                ;;
            geectl__subcmd__changelog,generate)
                cmd="geectl__subcmd__changelog__subcmd__generate"
                ;;
            geectl__subcmd__changelog,help)
                cmd="geectl__subcmd__changelog__subcmd__help"
                ;;
            geectl__subcmd__changelog,identity)
                cmd="geectl__subcmd__changelog__subcmd__identity"
                ;;
            geectl__subcmd__changelog,validate)
                cmd="geectl__subcmd__changelog__subcmd__validate"
                ;;
            geectl__subcmd__changelog__subcmd__help,generate)
                cmd="geectl__subcmd__changelog__subcmd__help__subcmd__generate"
                ;;
            geectl__subcmd__changelog__subcmd__help,help)
                cmd="geectl__subcmd__changelog__subcmd__help__subcmd__help"
                ;;
            geectl__subcmd__changelog__subcmd__help,identity)
                cmd="geectl__subcmd__changelog__subcmd__help__subcmd__identity"
                ;;
            geectl__subcmd__changelog__subcmd__help,validate)
                cmd="geectl__subcmd__changelog__subcmd__help__subcmd__validate"
                ;;
            geectl__subcmd__config,help)
                cmd="geectl__subcmd__config__subcmd__help"
                ;;
            geectl__subcmd__config,import-oscrc)
                cmd="geectl__subcmd__config__subcmd__import__subcmd__oscrc"
                ;;
            geectl__subcmd__config,init)
                cmd="geectl__subcmd__config__subcmd__init"
                ;;
            geectl__subcmd__config,show)
                cmd="geectl__subcmd__config__subcmd__show"
                ;;
            geectl__subcmd__config__subcmd__help,help)
                cmd="geectl__subcmd__config__subcmd__help__subcmd__help"
                ;;
            geectl__subcmd__config__subcmd__help,import-oscrc)
                cmd="geectl__subcmd__config__subcmd__help__subcmd__import__subcmd__oscrc"
                ;;
            geectl__subcmd__config__subcmd__help,init)
                cmd="geectl__subcmd__config__subcmd__help__subcmd__init"
                ;;
            geectl__subcmd__config__subcmd__help,show)
                cmd="geectl__subcmd__config__subcmd__help__subcmd__show"
                ;;
            geectl__subcmd__gitea,branch-get)
                cmd="geectl__subcmd__gitea__subcmd__branch__subcmd__get"
                ;;
            geectl__subcmd__gitea,branch-head)
                cmd="geectl__subcmd__gitea__subcmd__branch__subcmd__head"
                ;;
            geectl__subcmd__gitea,branch-list)
                cmd="geectl__subcmd__gitea__subcmd__branch__subcmd__list"
                ;;
            geectl__subcmd__gitea,cleanup-forks)
                cmd="geectl__subcmd__gitea__subcmd__cleanup__subcmd__forks"
                ;;
            geectl__subcmd__gitea,commit-statuses)
                cmd="geectl__subcmd__gitea__subcmd__commit__subcmd__statuses"
                ;;
            geectl__subcmd__gitea,help)
                cmd="geectl__subcmd__gitea__subcmd__help"
                ;;
            geectl__subcmd__gitea,maintainers)
                cmd="geectl__subcmd__gitea__subcmd__maintainers"
                ;;
            geectl__subcmd__gitea,raw-file)
                cmd="geectl__subcmd__gitea__subcmd__raw__subcmd__file"
                ;;
            geectl__subcmd__gitea,repo-create)
                cmd="geectl__subcmd__gitea__subcmd__repo__subcmd__create"
                ;;
            geectl__subcmd__gitea,repo-delete)
                cmd="geectl__subcmd__gitea__subcmd__repo__subcmd__delete"
                ;;
            geectl__subcmd__gitea,repo-fork)
                cmd="geectl__subcmd__gitea__subcmd__repo__subcmd__fork"
                ;;
            geectl__subcmd__gitea,repo-get)
                cmd="geectl__subcmd__gitea__subcmd__repo__subcmd__get"
                ;;
            geectl__subcmd__gitea,repo-list)
                cmd="geectl__subcmd__gitea__subcmd__repo__subcmd__list"
                ;;
            geectl__subcmd__gitea,repo-search)
                cmd="geectl__subcmd__gitea__subcmd__repo__subcmd__search"
                ;;
            geectl__subcmd__gitea,whoami)
                cmd="geectl__subcmd__gitea__subcmd__whoami"
                ;;
            geectl__subcmd__gitea__subcmd__help,branch-get)
                cmd="geectl__subcmd__gitea__subcmd__help__subcmd__branch__subcmd__get"
                ;;
            geectl__subcmd__gitea__subcmd__help,branch-head)
                cmd="geectl__subcmd__gitea__subcmd__help__subcmd__branch__subcmd__head"
                ;;
            geectl__subcmd__gitea__subcmd__help,branch-list)
                cmd="geectl__subcmd__gitea__subcmd__help__subcmd__branch__subcmd__list"
                ;;
            geectl__subcmd__gitea__subcmd__help,cleanup-forks)
                cmd="geectl__subcmd__gitea__subcmd__help__subcmd__cleanup__subcmd__forks"
                ;;
            geectl__subcmd__gitea__subcmd__help,commit-statuses)
                cmd="geectl__subcmd__gitea__subcmd__help__subcmd__commit__subcmd__statuses"
                ;;
            geectl__subcmd__gitea__subcmd__help,help)
                cmd="geectl__subcmd__gitea__subcmd__help__subcmd__help"
                ;;
            geectl__subcmd__gitea__subcmd__help,maintainers)
                cmd="geectl__subcmd__gitea__subcmd__help__subcmd__maintainers"
                ;;
            geectl__subcmd__gitea__subcmd__help,raw-file)
                cmd="geectl__subcmd__gitea__subcmd__help__subcmd__raw__subcmd__file"
                ;;
            geectl__subcmd__gitea__subcmd__help,repo-create)
                cmd="geectl__subcmd__gitea__subcmd__help__subcmd__repo__subcmd__create"
                ;;
            geectl__subcmd__gitea__subcmd__help,repo-delete)
                cmd="geectl__subcmd__gitea__subcmd__help__subcmd__repo__subcmd__delete"
                ;;
            geectl__subcmd__gitea__subcmd__help,repo-fork)
                cmd="geectl__subcmd__gitea__subcmd__help__subcmd__repo__subcmd__fork"
                ;;
            geectl__subcmd__gitea__subcmd__help,repo-get)
                cmd="geectl__subcmd__gitea__subcmd__help__subcmd__repo__subcmd__get"
                ;;
            geectl__subcmd__gitea__subcmd__help,repo-list)
                cmd="geectl__subcmd__gitea__subcmd__help__subcmd__repo__subcmd__list"
                ;;
            geectl__subcmd__gitea__subcmd__help,repo-search)
                cmd="geectl__subcmd__gitea__subcmd__help__subcmd__repo__subcmd__search"
                ;;
            geectl__subcmd__gitea__subcmd__help,whoami)
                cmd="geectl__subcmd__gitea__subcmd__help__subcmd__whoami"
                ;;
            geectl__subcmd__help,add)
                cmd="geectl__subcmd__help__subcmd__add"
                ;;
            geectl__subcmd__help,addremove)
                cmd="geectl__subcmd__help__subcmd__addremove"
                ;;
            geectl__subcmd__help,branch)
                cmd="geectl__subcmd__help__subcmd__branch"
                ;;
            geectl__subcmd__help,build)
                cmd="geectl__subcmd__help__subcmd__build"
                ;;
            geectl__subcmd__help,changelog)
                cmd="geectl__subcmd__help__subcmd__changelog"
                ;;
            geectl__subcmd__help,checkout)
                cmd="geectl__subcmd__help__subcmd__checkout"
                ;;
            geectl__subcmd__help,commit)
                cmd="geectl__subcmd__help__subcmd__commit"
                ;;
            geectl__subcmd__help,config)
                cmd="geectl__subcmd__help__subcmd__config"
                ;;
            geectl__subcmd__help,diff)
                cmd="geectl__subcmd__help__subcmd__diff"
                ;;
            geectl__subcmd__help,gitea)
                cmd="geectl__subcmd__help__subcmd__gitea"
                ;;
            geectl__subcmd__help,help)
                cmd="geectl__subcmd__help__subcmd__help"
                ;;
            geectl__subcmd__help,local-build)
                cmd="geectl__subcmd__help__subcmd__local__subcmd__build"
                ;;
            geectl__subcmd__help,maintenance)
                cmd="geectl__subcmd__help__subcmd__maintenance"
                ;;
            geectl__subcmd__help,meta)
                cmd="geectl__subcmd__help__subcmd__meta"
                ;;
            geectl__subcmd__help,person)
                cmd="geectl__subcmd__help__subcmd__person"
                ;;
            geectl__subcmd__help,prjconf)
                cmd="geectl__subcmd__help__subcmd__prjconf"
                ;;
            geectl__subcmd__help,pull)
                cmd="geectl__subcmd__help__subcmd__pull"
                ;;
            geectl__subcmd__help,pull-request)
                cmd="geectl__subcmd__help__subcmd__pull__subcmd__request"
                ;;
            geectl__subcmd__help,remote)
                cmd="geectl__subcmd__help__subcmd__remote"
                ;;
            geectl__subcmd__help,request)
                cmd="geectl__subcmd__help__subcmd__request"
                ;;
            geectl__subcmd__help,resolve)
                cmd="geectl__subcmd__help__subcmd__resolve"
                ;;
            geectl__subcmd__help,rm)
                cmd="geectl__subcmd__help__subcmd__rm"
                ;;
            geectl__subcmd__help,service)
                cmd="geectl__subcmd__help__subcmd__service"
                ;;
            geectl__subcmd__help,status)
                cmd="geectl__subcmd__help__subcmd__status"
                ;;
            geectl__subcmd__help,update)
                cmd="geectl__subcmd__help__subcmd__update"
                ;;
            geectl__subcmd__help,version)
                cmd="geectl__subcmd__help__subcmd__version"
                ;;
            geectl__subcmd__help__subcmd__branch,branch-package)
                cmd="geectl__subcmd__help__subcmd__branch__subcmd__branch__subcmd__package"
                ;;
            geectl__subcmd__help__subcmd__branch,copy-package)
                cmd="geectl__subcmd__help__subcmd__branch__subcmd__copy__subcmd__package"
                ;;
            geectl__subcmd__help__subcmd__branch,copy-project)
                cmd="geectl__subcmd__help__subcmd__branch__subcmd__copy__subcmd__project"
                ;;
            geectl__subcmd__help__subcmd__branch,create)
                cmd="geectl__subcmd__help__subcmd__branch__subcmd__create"
                ;;
            geectl__subcmd__help__subcmd__branch,detach-branch)
                cmd="geectl__subcmd__help__subcmd__branch__subcmd__detach__subcmd__branch"
                ;;
            geectl__subcmd__help__subcmd__branch,link-package)
                cmd="geectl__subcmd__help__subcmd__branch__subcmd__link__subcmd__package"
                ;;
            geectl__subcmd__help__subcmd__branch,link-to-branch)
                cmd="geectl__subcmd__help__subcmd__branch__subcmd__link__subcmd__to__subcmd__branch"
                ;;
            geectl__subcmd__help__subcmd__branch,repair-link)
                cmd="geectl__subcmd__help__subcmd__branch__subcmd__repair__subcmd__link"
                ;;
            geectl__subcmd__help__subcmd__branch,set-devel-project)
                cmd="geectl__subcmd__help__subcmd__branch__subcmd__set__subcmd__devel__subcmd__project"
                ;;
            geectl__subcmd__help__subcmd__branch,set-link-rev)
                cmd="geectl__subcmd__help__subcmd__branch__subcmd__set__subcmd__link__subcmd__rev"
                ;;
            geectl__subcmd__help__subcmd__branch,set-link-rev-project)
                cmd="geectl__subcmd__help__subcmd__branch__subcmd__set__subcmd__link__subcmd__rev__subcmd__project"
                ;;
            geectl__subcmd__help__subcmd__build,abort-build)
                cmd="geectl__subcmd__help__subcmd__build__subcmd__abort__subcmd__build"
                ;;
            geectl__subcmd__help__subcmd__build,build-history)
                cmd="geectl__subcmd__help__subcmd__build__subcmd__build__subcmd__history"
                ;;
            geectl__subcmd__help__subcmd__build,build-log)
                cmd="geectl__subcmd__help__subcmd__build__subcmd__build__subcmd__log"
                ;;
            geectl__subcmd__help__subcmd__build,build-status)
                cmd="geectl__subcmd__help__subcmd__build__subcmd__build__subcmd__status"
                ;;
            geectl__subcmd__help__subcmd__build,buildconfig)
                cmd="geectl__subcmd__help__subcmd__build__subcmd__buildconfig"
                ;;
            geectl__subcmd__help__subcmd__build,buildinfo)
                cmd="geectl__subcmd__help__subcmd__build__subcmd__buildinfo"
                ;;
            geectl__subcmd__help__subcmd__build,check-constraints)
                cmd="geectl__subcmd__help__subcmd__build__subcmd__check__subcmd__constraints"
                ;;
            geectl__subcmd__help__subcmd__build,dependson)
                cmd="geectl__subcmd__help__subcmd__build__subcmd__dependson"
                ;;
            geectl__subcmd__help__subcmd__build,download-binary)
                cmd="geectl__subcmd__help__subcmd__build__subcmd__download__subcmd__binary"
                ;;
            geectl__subcmd__help__subcmd__build,fileinfo)
                cmd="geectl__subcmd__help__subcmd__build__subcmd__fileinfo"
                ;;
            geectl__subcmd__help__subcmd__build,job-history)
                cmd="geectl__subcmd__help__subcmd__build__subcmd__job__subcmd__history"
                ;;
            geectl__subcmd__help__subcmd__build,job-status)
                cmd="geectl__subcmd__help__subcmd__build__subcmd__job__subcmd__status"
                ;;
            geectl__subcmd__help__subcmd__build,list-binaries)
                cmd="geectl__subcmd__help__subcmd__build__subcmd__list__subcmd__binaries"
                ;;
            geectl__subcmd__help__subcmd__build,list-repositories)
                cmd="geectl__subcmd__help__subcmd__build__subcmd__list__subcmd__repositories"
                ;;
            geectl__subcmd__help__subcmd__build,rebuild-package)
                cmd="geectl__subcmd__help__subcmd__build__subcmd__rebuild__subcmd__package"
                ;;
            geectl__subcmd__help__subcmd__build,restart-build)
                cmd="geectl__subcmd__help__subcmd__build__subcmd__restart__subcmd__build"
                ;;
            geectl__subcmd__help__subcmd__build,results)
                cmd="geectl__subcmd__help__subcmd__build__subcmd__results"
                ;;
            geectl__subcmd__help__subcmd__build,wait-for-results)
                cmd="geectl__subcmd__help__subcmd__build__subcmd__wait__subcmd__for__subcmd__results"
                ;;
            geectl__subcmd__help__subcmd__build,whatdependson)
                cmd="geectl__subcmd__help__subcmd__build__subcmd__whatdependson"
                ;;
            geectl__subcmd__help__subcmd__build,wipe-binaries)
                cmd="geectl__subcmd__help__subcmd__build__subcmd__wipe__subcmd__binaries"
                ;;
            geectl__subcmd__help__subcmd__changelog,generate)
                cmd="geectl__subcmd__help__subcmd__changelog__subcmd__generate"
                ;;
            geectl__subcmd__help__subcmd__changelog,identity)
                cmd="geectl__subcmd__help__subcmd__changelog__subcmd__identity"
                ;;
            geectl__subcmd__help__subcmd__changelog,validate)
                cmd="geectl__subcmd__help__subcmd__changelog__subcmd__validate"
                ;;
            geectl__subcmd__help__subcmd__config,import-oscrc)
                cmd="geectl__subcmd__help__subcmd__config__subcmd__import__subcmd__oscrc"
                ;;
            geectl__subcmd__help__subcmd__config,init)
                cmd="geectl__subcmd__help__subcmd__config__subcmd__init"
                ;;
            geectl__subcmd__help__subcmd__config,show)
                cmd="geectl__subcmd__help__subcmd__config__subcmd__show"
                ;;
            geectl__subcmd__help__subcmd__gitea,branch-get)
                cmd="geectl__subcmd__help__subcmd__gitea__subcmd__branch__subcmd__get"
                ;;
            geectl__subcmd__help__subcmd__gitea,branch-head)
                cmd="geectl__subcmd__help__subcmd__gitea__subcmd__branch__subcmd__head"
                ;;
            geectl__subcmd__help__subcmd__gitea,branch-list)
                cmd="geectl__subcmd__help__subcmd__gitea__subcmd__branch__subcmd__list"
                ;;
            geectl__subcmd__help__subcmd__gitea,cleanup-forks)
                cmd="geectl__subcmd__help__subcmd__gitea__subcmd__cleanup__subcmd__forks"
                ;;
            geectl__subcmd__help__subcmd__gitea,commit-statuses)
                cmd="geectl__subcmd__help__subcmd__gitea__subcmd__commit__subcmd__statuses"
                ;;
            geectl__subcmd__help__subcmd__gitea,maintainers)
                cmd="geectl__subcmd__help__subcmd__gitea__subcmd__maintainers"
                ;;
            geectl__subcmd__help__subcmd__gitea,raw-file)
                cmd="geectl__subcmd__help__subcmd__gitea__subcmd__raw__subcmd__file"
                ;;
            geectl__subcmd__help__subcmd__gitea,repo-create)
                cmd="geectl__subcmd__help__subcmd__gitea__subcmd__repo__subcmd__create"
                ;;
            geectl__subcmd__help__subcmd__gitea,repo-delete)
                cmd="geectl__subcmd__help__subcmd__gitea__subcmd__repo__subcmd__delete"
                ;;
            geectl__subcmd__help__subcmd__gitea,repo-fork)
                cmd="geectl__subcmd__help__subcmd__gitea__subcmd__repo__subcmd__fork"
                ;;
            geectl__subcmd__help__subcmd__gitea,repo-get)
                cmd="geectl__subcmd__help__subcmd__gitea__subcmd__repo__subcmd__get"
                ;;
            geectl__subcmd__help__subcmd__gitea,repo-list)
                cmd="geectl__subcmd__help__subcmd__gitea__subcmd__repo__subcmd__list"
                ;;
            geectl__subcmd__help__subcmd__gitea,repo-search)
                cmd="geectl__subcmd__help__subcmd__gitea__subcmd__repo__subcmd__search"
                ;;
            geectl__subcmd__help__subcmd__gitea,whoami)
                cmd="geectl__subcmd__help__subcmd__gitea__subcmd__whoami"
                ;;
            geectl__subcmd__help__subcmd__local__subcmd__build,build)
                cmd="geectl__subcmd__help__subcmd__local__subcmd__build__subcmd__build"
                ;;
            geectl__subcmd__help__subcmd__local__subcmd__build,chroot)
                cmd="geectl__subcmd__help__subcmd__local__subcmd__build__subcmd__chroot"
                ;;
            geectl__subcmd__help__subcmd__local__subcmd__build,log)
                cmd="geectl__subcmd__help__subcmd__local__subcmd__build__subcmd__log"
                ;;
            geectl__subcmd__help__subcmd__local__subcmd__build,periodic-install)
                cmd="geectl__subcmd__help__subcmd__local__subcmd__build__subcmd__periodic__subcmd__install"
                ;;
            geectl__subcmd__help__subcmd__local__subcmd__build,periodic-status)
                cmd="geectl__subcmd__help__subcmd__local__subcmd__build__subcmd__periodic__subcmd__status"
                ;;
            geectl__subcmd__help__subcmd__local__subcmd__build,periodic-uninstall)
                cmd="geectl__subcmd__help__subcmd__local__subcmd__build__subcmd__periodic__subcmd__uninstall"
                ;;
            geectl__subcmd__help__subcmd__local__subcmd__build,precommit)
                cmd="geectl__subcmd__help__subcmd__local__subcmd__build__subcmd__precommit"
                ;;
            geectl__subcmd__help__subcmd__local__subcmd__build,prune)
                cmd="geectl__subcmd__help__subcmd__local__subcmd__build__subcmd__prune"
                ;;
            geectl__subcmd__help__subcmd__local__subcmd__build,service-run)
                cmd="geectl__subcmd__help__subcmd__local__subcmd__build__subcmd__service__subcmd__run"
                ;;
            geectl__subcmd__help__subcmd__local__subcmd__build,spec-cleaner)
                cmd="geectl__subcmd__help__subcmd__local__subcmd__build__subcmd__spec__subcmd__cleaner"
                ;;
            geectl__subcmd__help__subcmd__local__subcmd__build,wipe)
                cmd="geectl__subcmd__help__subcmd__local__subcmd__build__subcmd__wipe"
                ;;
            geectl__subcmd__help__subcmd__maintenance,maintained-instances)
                cmd="geectl__subcmd__help__subcmd__maintenance__subcmd__maintained__subcmd__instances"
                ;;
            geectl__subcmd__help__subcmd__maintenance,maintenance-branch)
                cmd="geectl__subcmd__help__subcmd__maintenance__subcmd__maintenance__subcmd__branch"
                ;;
            geectl__subcmd__help__subcmd__meta,set)
                cmd="geectl__subcmd__help__subcmd__meta__subcmd__set"
                ;;
            geectl__subcmd__help__subcmd__person,search)
                cmd="geectl__subcmd__help__subcmd__person__subcmd__search"
                ;;
            geectl__subcmd__help__subcmd__prjconf,clean)
                cmd="geectl__subcmd__help__subcmd__prjconf__subcmd__clean"
                ;;
            geectl__subcmd__help__subcmd__prjconf,show)
                cmd="geectl__subcmd__help__subcmd__prjconf__subcmd__show"
                ;;
            geectl__subcmd__help__subcmd__prjconf,validate)
                cmd="geectl__subcmd__help__subcmd__prjconf__subcmd__validate"
                ;;
            geectl__subcmd__help__subcmd__pull__subcmd__request,approve)
                cmd="geectl__subcmd__help__subcmd__pull__subcmd__request__subcmd__approve"
                ;;
            geectl__subcmd__help__subcmd__pull__subcmd__request,build-comment)
                cmd="geectl__subcmd__help__subcmd__pull__subcmd__request__subcmd__build__subcmd__comment"
                ;;
            geectl__subcmd__help__subcmd__pull__subcmd__request,cancel-review-request)
                cmd="geectl__subcmd__help__subcmd__pull__subcmd__request__subcmd__cancel__subcmd__review__subcmd__request"
                ;;
            geectl__subcmd__help__subcmd__pull__subcmd__request,cleanup)
                cmd="geectl__subcmd__help__subcmd__pull__subcmd__request__subcmd__cleanup"
                ;;
            geectl__subcmd__help__subcmd__pull__subcmd__request,comment)
                cmd="geectl__subcmd__help__subcmd__pull__subcmd__request__subcmd__comment"
                ;;
            geectl__subcmd__help__subcmd__pull__subcmd__request,comments)
                cmd="geectl__subcmd__help__subcmd__pull__subcmd__request__subcmd__comments"
                ;;
            geectl__subcmd__help__subcmd__pull__subcmd__request,decline)
                cmd="geectl__subcmd__help__subcmd__pull__subcmd__request__subcmd__decline"
                ;;
            geectl__subcmd__help__subcmd__pull__subcmd__request,diff)
                cmd="geectl__subcmd__help__subcmd__pull__subcmd__request__subcmd__diff"
                ;;
            geectl__subcmd__help__subcmd__pull__subcmd__request,merge)
                cmd="geectl__subcmd__help__subcmd__pull__subcmd__request__subcmd__merge"
                ;;
            geectl__subcmd__help__subcmd__pull__subcmd__request,reopen)
                cmd="geectl__subcmd__help__subcmd__pull__subcmd__request__subcmd__reopen"
                ;;
            geectl__subcmd__help__subcmd__pull__subcmd__request,request-changes)
                cmd="geectl__subcmd__help__subcmd__pull__subcmd__request__subcmd__request__subcmd__changes"
                ;;
            geectl__subcmd__help__subcmd__pull__subcmd__request,search)
                cmd="geectl__subcmd__help__subcmd__pull__subcmd__request__subcmd__search"
                ;;
            geectl__subcmd__help__subcmd__pull__subcmd__request,show)
                cmd="geectl__subcmd__help__subcmd__pull__subcmd__request__subcmd__show"
                ;;
            geectl__subcmd__help__subcmd__pull__subcmd__request,submit)
                cmd="geectl__subcmd__help__subcmd__pull__subcmd__request__subcmd__submit"
                ;;
            geectl__subcmd__help__subcmd__pull__subcmd__request,withdraw)
                cmd="geectl__subcmd__help__subcmd__pull__subcmd__request__subcmd__withdraw"
                ;;
            geectl__subcmd__help__subcmd__remote,add-channels)
                cmd="geectl__subcmd__help__subcmd__remote__subcmd__add__subcmd__channels"
                ;;
            geectl__subcmd__help__subcmd__remote,add-containers)
                cmd="geectl__subcmd__help__subcmd__remote__subcmd__add__subcmd__containers"
                ;;
            geectl__subcmd__help__subcmd__remote,aggregate-package)
                cmd="geectl__subcmd__help__subcmd__remote__subcmd__aggregate__subcmd__package"
                ;;
            geectl__subcmd__help__subcmd__remote,cat)
                cmd="geectl__subcmd__help__subcmd__remote__subcmd__cat"
                ;;
            geectl__subcmd__help__subcmd__remote,count-scmsync-projects)
                cmd="geectl__subcmd__help__subcmd__remote__subcmd__count__subcmd__scmsync__subcmd__projects"
                ;;
            geectl__subcmd__help__subcmd__remote,create-package)
                cmd="geectl__subcmd__help__subcmd__remote__subcmd__create__subcmd__package"
                ;;
            geectl__subcmd__help__subcmd__remote,create-project)
                cmd="geectl__subcmd__help__subcmd__remote__subcmd__create__subcmd__project"
                ;;
            geectl__subcmd__help__subcmd__remote,create-token)
                cmd="geectl__subcmd__help__subcmd__remote__subcmd__create__subcmd__token"
                ;;
            geectl__subcmd__help__subcmd__remote,delete-file)
                cmd="geectl__subcmd__help__subcmd__remote__subcmd__delete__subcmd__file"
                ;;
            geectl__subcmd__help__subcmd__remote,delete-link)
                cmd="geectl__subcmd__help__subcmd__remote__subcmd__delete__subcmd__link"
                ;;
            geectl__subcmd__help__subcmd__remote,delete-package)
                cmd="geectl__subcmd__help__subcmd__remote__subcmd__delete__subcmd__package"
                ;;
            geectl__subcmd__help__subcmd__remote,delete-project)
                cmd="geectl__subcmd__help__subcmd__remote__subcmd__delete__subcmd__project"
                ;;
            geectl__subcmd__help__subcmd__remote,devel-project)
                cmd="geectl__subcmd__help__subcmd__remote__subcmd__devel__subcmd__project"
                ;;
            geectl__subcmd__help__subcmd__remote,diff-packages)
                cmd="geectl__subcmd__help__subcmd__remote__subcmd__diff__subcmd__packages"
                ;;
            geectl__subcmd__help__subcmd__remote,download)
                cmd="geectl__subcmd__help__subcmd__remote__subcmd__download"
                ;;
            geectl__subcmd__help__subcmd__remote,link-target)
                cmd="geectl__subcmd__help__subcmd__remote__subcmd__link__subcmd__target"
                ;;
            geectl__subcmd__help__subcmd__remote,list-files)
                cmd="geectl__subcmd__help__subcmd__remote__subcmd__list__subcmd__files"
                ;;
            geectl__subcmd__help__subcmd__remote,list-packages)
                cmd="geectl__subcmd__help__subcmd__remote__subcmd__list__subcmd__packages"
                ;;
            geectl__subcmd__help__subcmd__remote,list-projects)
                cmd="geectl__subcmd__help__subcmd__remote__subcmd__list__subcmd__projects"
                ;;
            geectl__subcmd__help__subcmd__remote,list-scmsync-projects)
                cmd="geectl__subcmd__help__subcmd__remote__subcmd__list__subcmd__scmsync__subcmd__projects"
                ;;
            geectl__subcmd__help__subcmd__remote,list-staging-projects)
                cmd="geectl__subcmd__help__subcmd__remote__subcmd__list__subcmd__staging__subcmd__projects"
                ;;
            geectl__subcmd__help__subcmd__remote,lock)
                cmd="geectl__subcmd__help__subcmd__remote__subcmd__lock"
                ;;
            geectl__subcmd__help__subcmd__remote,maintainers)
                cmd="geectl__subcmd__help__subcmd__remote__subcmd__maintainers"
                ;;
            geectl__subcmd__help__subcmd__remote,owners)
                cmd="geectl__subcmd__help__subcmd__remote__subcmd__owners"
                ;;
            geectl__subcmd__help__subcmd__remote,owners-for-binary)
                cmd="geectl__subcmd__help__subcmd__remote__subcmd__owners__subcmd__for__subcmd__binary"
                ;;
            geectl__subcmd__help__subcmd__remote,owners-for-package)
                cmd="geectl__subcmd__help__subcmd__remote__subcmd__owners__subcmd__for__subcmd__package"
                ;;
            geectl__subcmd__help__subcmd__remote,package-log)
                cmd="geectl__subcmd__help__subcmd__remote__subcmd__package__subcmd__log"
                ;;
            geectl__subcmd__help__subcmd__remote,package-meta)
                cmd="geectl__subcmd__help__subcmd__remote__subcmd__package__subcmd__meta"
                ;;
            geectl__subcmd__help__subcmd__remote,project-diff)
                cmd="geectl__subcmd__help__subcmd__remote__subcmd__project__subcmd__diff"
                ;;
            geectl__subcmd__help__subcmd__remote,project-meta)
                cmd="geectl__subcmd__help__subcmd__remote__subcmd__project__subcmd__meta"
                ;;
            geectl__subcmd__help__subcmd__remote,release-package)
                cmd="geectl__subcmd__help__subcmd__remote__subcmd__release__subcmd__package"
                ;;
            geectl__subcmd__help__subcmd__remote,release-project)
                cmd="geectl__subcmd__help__subcmd__remote__subcmd__release__subcmd__project"
                ;;
            geectl__subcmd__help__subcmd__remote,repo-add)
                cmd="geectl__subcmd__help__subcmd__remote__subcmd__repo__subcmd__add"
                ;;
            geectl__subcmd__help__subcmd__remote,repo-remove)
                cmd="geectl__subcmd__help__subcmd__remote__subcmd__repo__subcmd__remove"
                ;;
            geectl__subcmd__help__subcmd__remote,revision-patch)
                cmd="geectl__subcmd__help__subcmd__remote__subcmd__revision__subcmd__patch"
                ;;
            geectl__subcmd__help__subcmd__remote,search-packages)
                cmd="geectl__subcmd__help__subcmd__remote__subcmd__search__subcmd__packages"
                ;;
            geectl__subcmd__help__subcmd__remote,search-packages-by-person)
                cmd="geectl__subcmd__help__subcmd__remote__subcmd__search__subcmd__packages__subcmd__by__subcmd__person"
                ;;
            geectl__subcmd__help__subcmd__remote,search-projects)
                cmd="geectl__subcmd__help__subcmd__remote__subcmd__search__subcmd__projects"
                ;;
            geectl__subcmd__help__subcmd__remote,search-projects-by-person)
                cmd="geectl__subcmd__help__subcmd__remote__subcmd__search__subcmd__projects__subcmd__by__subcmd__person"
                ;;
            geectl__subcmd__help__subcmd__remote,set-package-meta)
                cmd="geectl__subcmd__help__subcmd__remote__subcmd__set__subcmd__package__subcmd__meta"
                ;;
            geectl__subcmd__help__subcmd__remote,set-project-meta)
                cmd="geectl__subcmd__help__subcmd__remote__subcmd__set__subcmd__project__subcmd__meta"
                ;;
            geectl__subcmd__help__subcmd__remote,staging-project)
                cmd="geectl__subcmd__help__subcmd__remote__subcmd__staging__subcmd__project"
                ;;
            geectl__subcmd__help__subcmd__remote,undelete-package)
                cmd="geectl__subcmd__help__subcmd__remote__subcmd__undelete__subcmd__package"
                ;;
            geectl__subcmd__help__subcmd__remote,undelete-project)
                cmd="geectl__subcmd__help__subcmd__remote__subcmd__undelete__subcmd__project"
                ;;
            geectl__subcmd__help__subcmd__remote,unlock)
                cmd="geectl__subcmd__help__subcmd__remote__subcmd__unlock"
                ;;
            geectl__subcmd__help__subcmd__remote,update-package-meta-from-spec)
                cmd="geectl__subcmd__help__subcmd__remote__subcmd__update__subcmd__package__subcmd__meta__subcmd__from__subcmd__spec"
                ;;
            geectl__subcmd__help__subcmd__remote,upload-file)
                cmd="geectl__subcmd__help__subcmd__remote__subcmd__upload__subcmd__file"
                ;;
            geectl__subcmd__help__subcmd__remote,whois)
                cmd="geectl__subcmd__help__subcmd__remote__subcmd__whois"
                ;;
            geectl__subcmd__help__subcmd__request,accept)
                cmd="geectl__subcmd__help__subcmd__request__subcmd__accept"
                ;;
            geectl__subcmd__help__subcmd__request,add-comment)
                cmd="geectl__subcmd__help__subcmd__request__subcmd__add__subcmd__comment"
                ;;
            geectl__subcmd__help__subcmd__request,add-review)
                cmd="geectl__subcmd__help__subcmd__request__subcmd__add__subcmd__review"
                ;;
            geectl__subcmd__help__subcmd__request,change-review-state)
                cmd="geectl__subcmd__help__subcmd__request__subcmd__change__subcmd__review__subcmd__state"
                ;;
            geectl__subcmd__help__subcmd__request,change-state)
                cmd="geectl__subcmd__help__subcmd__request__subcmd__change__subcmd__state"
                ;;
            geectl__subcmd__help__subcmd__request,decline)
                cmd="geectl__subcmd__help__subcmd__request__subcmd__decline"
                ;;
            geectl__subcmd__help__subcmd__request,diff)
                cmd="geectl__subcmd__help__subcmd__request__subcmd__diff"
                ;;
            geectl__subcmd__help__subcmd__request,file)
                cmd="geectl__subcmd__help__subcmd__request__subcmd__file"
                ;;
            geectl__subcmd__help__subcmd__request,file-add-role)
                cmd="geectl__subcmd__help__subcmd__request__subcmd__file__subcmd__add__subcmd__role"
                ;;
            geectl__subcmd__help__subcmd__request,file-change-devel)
                cmd="geectl__subcmd__help__subcmd__request__subcmd__file__subcmd__change__subcmd__devel"
                ;;
            geectl__subcmd__help__subcmd__request,file-delete)
                cmd="geectl__subcmd__help__subcmd__request__subcmd__file__subcmd__delete"
                ;;
            geectl__subcmd__help__subcmd__request,file-maintenance-incident)
                cmd="geectl__subcmd__help__subcmd__request__subcmd__file__subcmd__maintenance__subcmd__incident"
                ;;
            geectl__subcmd__help__subcmd__request,list)
                cmd="geectl__subcmd__help__subcmd__request__subcmd__list"
                ;;
            geectl__subcmd__help__subcmd__request,list-comments)
                cmd="geectl__subcmd__help__subcmd__request__subcmd__list__subcmd__comments"
                ;;
            geectl__subcmd__help__subcmd__request,log)
                cmd="geectl__subcmd__help__subcmd__request__subcmd__log"
                ;;
            geectl__subcmd__help__subcmd__request,refile)
                cmd="geectl__subcmd__help__subcmd__request__subcmd__refile"
                ;;
            geectl__subcmd__help__subcmd__request,refile-maintenance-incident)
                cmd="geectl__subcmd__help__subcmd__request__subcmd__refile__subcmd__maintenance__subcmd__incident"
                ;;
            geectl__subcmd__help__subcmd__request,revoke)
                cmd="geectl__subcmd__help__subcmd__request__subcmd__revoke"
                ;;
            geectl__subcmd__help__subcmd__request,search)
                cmd="geectl__subcmd__help__subcmd__request__subcmd__search"
                ;;
            geectl__subcmd__help__subcmd__request,show)
                cmd="geectl__subcmd__help__subcmd__request__subcmd__show"
                ;;
            geectl__subcmd__help__subcmd__request,supersede)
                cmd="geectl__subcmd__help__subcmd__request__subcmd__supersede"
                ;;
            geectl__subcmd__help__subcmd__request,try-change-state)
                cmd="geectl__subcmd__help__subcmd__request__subcmd__try__subcmd__change__subcmd__state"
                ;;
            geectl__subcmd__help__subcmd__service,list)
                cmd="geectl__subcmd__help__subcmd__service__subcmd__list"
                ;;
            geectl__subcmd__help__subcmd__service,state)
                cmd="geectl__subcmd__help__subcmd__service__subcmd__state"
                ;;
            geectl__subcmd__help__subcmd__service,trigger)
                cmd="geectl__subcmd__help__subcmd__service__subcmd__trigger"
                ;;
            geectl__subcmd__local__subcmd__build,build)
                cmd="geectl__subcmd__local__subcmd__build__subcmd__build"
                ;;
            geectl__subcmd__local__subcmd__build,chroot)
                cmd="geectl__subcmd__local__subcmd__build__subcmd__chroot"
                ;;
            geectl__subcmd__local__subcmd__build,help)
                cmd="geectl__subcmd__local__subcmd__build__subcmd__help"
                ;;
            geectl__subcmd__local__subcmd__build,log)
                cmd="geectl__subcmd__local__subcmd__build__subcmd__log"
                ;;
            geectl__subcmd__local__subcmd__build,periodic-install)
                cmd="geectl__subcmd__local__subcmd__build__subcmd__periodic__subcmd__install"
                ;;
            geectl__subcmd__local__subcmd__build,periodic-status)
                cmd="geectl__subcmd__local__subcmd__build__subcmd__periodic__subcmd__status"
                ;;
            geectl__subcmd__local__subcmd__build,periodic-uninstall)
                cmd="geectl__subcmd__local__subcmd__build__subcmd__periodic__subcmd__uninstall"
                ;;
            geectl__subcmd__local__subcmd__build,precommit)
                cmd="geectl__subcmd__local__subcmd__build__subcmd__precommit"
                ;;
            geectl__subcmd__local__subcmd__build,prune)
                cmd="geectl__subcmd__local__subcmd__build__subcmd__prune"
                ;;
            geectl__subcmd__local__subcmd__build,service-run)
                cmd="geectl__subcmd__local__subcmd__build__subcmd__service__subcmd__run"
                ;;
            geectl__subcmd__local__subcmd__build,spec-cleaner)
                cmd="geectl__subcmd__local__subcmd__build__subcmd__spec__subcmd__cleaner"
                ;;
            geectl__subcmd__local__subcmd__build,wipe)
                cmd="geectl__subcmd__local__subcmd__build__subcmd__wipe"
                ;;
            geectl__subcmd__local__subcmd__build__subcmd__help,build)
                cmd="geectl__subcmd__local__subcmd__build__subcmd__help__subcmd__build"
                ;;
            geectl__subcmd__local__subcmd__build__subcmd__help,chroot)
                cmd="geectl__subcmd__local__subcmd__build__subcmd__help__subcmd__chroot"
                ;;
            geectl__subcmd__local__subcmd__build__subcmd__help,help)
                cmd="geectl__subcmd__local__subcmd__build__subcmd__help__subcmd__help"
                ;;
            geectl__subcmd__local__subcmd__build__subcmd__help,log)
                cmd="geectl__subcmd__local__subcmd__build__subcmd__help__subcmd__log"
                ;;
            geectl__subcmd__local__subcmd__build__subcmd__help,periodic-install)
                cmd="geectl__subcmd__local__subcmd__build__subcmd__help__subcmd__periodic__subcmd__install"
                ;;
            geectl__subcmd__local__subcmd__build__subcmd__help,periodic-status)
                cmd="geectl__subcmd__local__subcmd__build__subcmd__help__subcmd__periodic__subcmd__status"
                ;;
            geectl__subcmd__local__subcmd__build__subcmd__help,periodic-uninstall)
                cmd="geectl__subcmd__local__subcmd__build__subcmd__help__subcmd__periodic__subcmd__uninstall"
                ;;
            geectl__subcmd__local__subcmd__build__subcmd__help,precommit)
                cmd="geectl__subcmd__local__subcmd__build__subcmd__help__subcmd__precommit"
                ;;
            geectl__subcmd__local__subcmd__build__subcmd__help,prune)
                cmd="geectl__subcmd__local__subcmd__build__subcmd__help__subcmd__prune"
                ;;
            geectl__subcmd__local__subcmd__build__subcmd__help,service-run)
                cmd="geectl__subcmd__local__subcmd__build__subcmd__help__subcmd__service__subcmd__run"
                ;;
            geectl__subcmd__local__subcmd__build__subcmd__help,spec-cleaner)
                cmd="geectl__subcmd__local__subcmd__build__subcmd__help__subcmd__spec__subcmd__cleaner"
                ;;
            geectl__subcmd__local__subcmd__build__subcmd__help,wipe)
                cmd="geectl__subcmd__local__subcmd__build__subcmd__help__subcmd__wipe"
                ;;
            geectl__subcmd__maintenance,help)
                cmd="geectl__subcmd__maintenance__subcmd__help"
                ;;
            geectl__subcmd__maintenance,maintained-instances)
                cmd="geectl__subcmd__maintenance__subcmd__maintained__subcmd__instances"
                ;;
            geectl__subcmd__maintenance,maintenance-branch)
                cmd="geectl__subcmd__maintenance__subcmd__maintenance__subcmd__branch"
                ;;
            geectl__subcmd__maintenance__subcmd__help,help)
                cmd="geectl__subcmd__maintenance__subcmd__help__subcmd__help"
                ;;
            geectl__subcmd__maintenance__subcmd__help,maintained-instances)
                cmd="geectl__subcmd__maintenance__subcmd__help__subcmd__maintained__subcmd__instances"
                ;;
            geectl__subcmd__maintenance__subcmd__help,maintenance-branch)
                cmd="geectl__subcmd__maintenance__subcmd__help__subcmd__maintenance__subcmd__branch"
                ;;
            geectl__subcmd__meta,help)
                cmd="geectl__subcmd__meta__subcmd__help"
                ;;
            geectl__subcmd__meta,set)
                cmd="geectl__subcmd__meta__subcmd__set"
                ;;
            geectl__subcmd__meta__subcmd__help,help)
                cmd="geectl__subcmd__meta__subcmd__help__subcmd__help"
                ;;
            geectl__subcmd__meta__subcmd__help,set)
                cmd="geectl__subcmd__meta__subcmd__help__subcmd__set"
                ;;
            geectl__subcmd__person,help)
                cmd="geectl__subcmd__person__subcmd__help"
                ;;
            geectl__subcmd__person,search)
                cmd="geectl__subcmd__person__subcmd__search"
                ;;
            geectl__subcmd__person__subcmd__help,help)
                cmd="geectl__subcmd__person__subcmd__help__subcmd__help"
                ;;
            geectl__subcmd__person__subcmd__help,search)
                cmd="geectl__subcmd__person__subcmd__help__subcmd__search"
                ;;
            geectl__subcmd__prjconf,clean)
                cmd="geectl__subcmd__prjconf__subcmd__clean"
                ;;
            geectl__subcmd__prjconf,help)
                cmd="geectl__subcmd__prjconf__subcmd__help"
                ;;
            geectl__subcmd__prjconf,show)
                cmd="geectl__subcmd__prjconf__subcmd__show"
                ;;
            geectl__subcmd__prjconf,validate)
                cmd="geectl__subcmd__prjconf__subcmd__validate"
                ;;
            geectl__subcmd__prjconf__subcmd__help,clean)
                cmd="geectl__subcmd__prjconf__subcmd__help__subcmd__clean"
                ;;
            geectl__subcmd__prjconf__subcmd__help,help)
                cmd="geectl__subcmd__prjconf__subcmd__help__subcmd__help"
                ;;
            geectl__subcmd__prjconf__subcmd__help,show)
                cmd="geectl__subcmd__prjconf__subcmd__help__subcmd__show"
                ;;
            geectl__subcmd__prjconf__subcmd__help,validate)
                cmd="geectl__subcmd__prjconf__subcmd__help__subcmd__validate"
                ;;
            geectl__subcmd__pull__subcmd__request,approve)
                cmd="geectl__subcmd__pull__subcmd__request__subcmd__approve"
                ;;
            geectl__subcmd__pull__subcmd__request,build-comment)
                cmd="geectl__subcmd__pull__subcmd__request__subcmd__build__subcmd__comment"
                ;;
            geectl__subcmd__pull__subcmd__request,cancel-review-request)
                cmd="geectl__subcmd__pull__subcmd__request__subcmd__cancel__subcmd__review__subcmd__request"
                ;;
            geectl__subcmd__pull__subcmd__request,cleanup)
                cmd="geectl__subcmd__pull__subcmd__request__subcmd__cleanup"
                ;;
            geectl__subcmd__pull__subcmd__request,comment)
                cmd="geectl__subcmd__pull__subcmd__request__subcmd__comment"
                ;;
            geectl__subcmd__pull__subcmd__request,comments)
                cmd="geectl__subcmd__pull__subcmd__request__subcmd__comments"
                ;;
            geectl__subcmd__pull__subcmd__request,decline)
                cmd="geectl__subcmd__pull__subcmd__request__subcmd__decline"
                ;;
            geectl__subcmd__pull__subcmd__request,diff)
                cmd="geectl__subcmd__pull__subcmd__request__subcmd__diff"
                ;;
            geectl__subcmd__pull__subcmd__request,help)
                cmd="geectl__subcmd__pull__subcmd__request__subcmd__help"
                ;;
            geectl__subcmd__pull__subcmd__request,merge)
                cmd="geectl__subcmd__pull__subcmd__request__subcmd__merge"
                ;;
            geectl__subcmd__pull__subcmd__request,reopen)
                cmd="geectl__subcmd__pull__subcmd__request__subcmd__reopen"
                ;;
            geectl__subcmd__pull__subcmd__request,request-changes)
                cmd="geectl__subcmd__pull__subcmd__request__subcmd__request__subcmd__changes"
                ;;
            geectl__subcmd__pull__subcmd__request,search)
                cmd="geectl__subcmd__pull__subcmd__request__subcmd__search"
                ;;
            geectl__subcmd__pull__subcmd__request,show)
                cmd="geectl__subcmd__pull__subcmd__request__subcmd__show"
                ;;
            geectl__subcmd__pull__subcmd__request,submit)
                cmd="geectl__subcmd__pull__subcmd__request__subcmd__submit"
                ;;
            geectl__subcmd__pull__subcmd__request,withdraw)
                cmd="geectl__subcmd__pull__subcmd__request__subcmd__withdraw"
                ;;
            geectl__subcmd__pull__subcmd__request__subcmd__help,approve)
                cmd="geectl__subcmd__pull__subcmd__request__subcmd__help__subcmd__approve"
                ;;
            geectl__subcmd__pull__subcmd__request__subcmd__help,build-comment)
                cmd="geectl__subcmd__pull__subcmd__request__subcmd__help__subcmd__build__subcmd__comment"
                ;;
            geectl__subcmd__pull__subcmd__request__subcmd__help,cancel-review-request)
                cmd="geectl__subcmd__pull__subcmd__request__subcmd__help__subcmd__cancel__subcmd__review__subcmd__request"
                ;;
            geectl__subcmd__pull__subcmd__request__subcmd__help,cleanup)
                cmd="geectl__subcmd__pull__subcmd__request__subcmd__help__subcmd__cleanup"
                ;;
            geectl__subcmd__pull__subcmd__request__subcmd__help,comment)
                cmd="geectl__subcmd__pull__subcmd__request__subcmd__help__subcmd__comment"
                ;;
            geectl__subcmd__pull__subcmd__request__subcmd__help,comments)
                cmd="geectl__subcmd__pull__subcmd__request__subcmd__help__subcmd__comments"
                ;;
            geectl__subcmd__pull__subcmd__request__subcmd__help,decline)
                cmd="geectl__subcmd__pull__subcmd__request__subcmd__help__subcmd__decline"
                ;;
            geectl__subcmd__pull__subcmd__request__subcmd__help,diff)
                cmd="geectl__subcmd__pull__subcmd__request__subcmd__help__subcmd__diff"
                ;;
            geectl__subcmd__pull__subcmd__request__subcmd__help,help)
                cmd="geectl__subcmd__pull__subcmd__request__subcmd__help__subcmd__help"
                ;;
            geectl__subcmd__pull__subcmd__request__subcmd__help,merge)
                cmd="geectl__subcmd__pull__subcmd__request__subcmd__help__subcmd__merge"
                ;;
            geectl__subcmd__pull__subcmd__request__subcmd__help,reopen)
                cmd="geectl__subcmd__pull__subcmd__request__subcmd__help__subcmd__reopen"
                ;;
            geectl__subcmd__pull__subcmd__request__subcmd__help,request-changes)
                cmd="geectl__subcmd__pull__subcmd__request__subcmd__help__subcmd__request__subcmd__changes"
                ;;
            geectl__subcmd__pull__subcmd__request__subcmd__help,search)
                cmd="geectl__subcmd__pull__subcmd__request__subcmd__help__subcmd__search"
                ;;
            geectl__subcmd__pull__subcmd__request__subcmd__help,show)
                cmd="geectl__subcmd__pull__subcmd__request__subcmd__help__subcmd__show"
                ;;
            geectl__subcmd__pull__subcmd__request__subcmd__help,submit)
                cmd="geectl__subcmd__pull__subcmd__request__subcmd__help__subcmd__submit"
                ;;
            geectl__subcmd__pull__subcmd__request__subcmd__help,withdraw)
                cmd="geectl__subcmd__pull__subcmd__request__subcmd__help__subcmd__withdraw"
                ;;
            geectl__subcmd__remote,add-channels)
                cmd="geectl__subcmd__remote__subcmd__add__subcmd__channels"
                ;;
            geectl__subcmd__remote,add-containers)
                cmd="geectl__subcmd__remote__subcmd__add__subcmd__containers"
                ;;
            geectl__subcmd__remote,aggregate-package)
                cmd="geectl__subcmd__remote__subcmd__aggregate__subcmd__package"
                ;;
            geectl__subcmd__remote,cat)
                cmd="geectl__subcmd__remote__subcmd__cat"
                ;;
            geectl__subcmd__remote,count-scmsync-projects)
                cmd="geectl__subcmd__remote__subcmd__count__subcmd__scmsync__subcmd__projects"
                ;;
            geectl__subcmd__remote,create-package)
                cmd="geectl__subcmd__remote__subcmd__create__subcmd__package"
                ;;
            geectl__subcmd__remote,create-project)
                cmd="geectl__subcmd__remote__subcmd__create__subcmd__project"
                ;;
            geectl__subcmd__remote,create-token)
                cmd="geectl__subcmd__remote__subcmd__create__subcmd__token"
                ;;
            geectl__subcmd__remote,delete-file)
                cmd="geectl__subcmd__remote__subcmd__delete__subcmd__file"
                ;;
            geectl__subcmd__remote,delete-link)
                cmd="geectl__subcmd__remote__subcmd__delete__subcmd__link"
                ;;
            geectl__subcmd__remote,delete-package)
                cmd="geectl__subcmd__remote__subcmd__delete__subcmd__package"
                ;;
            geectl__subcmd__remote,delete-project)
                cmd="geectl__subcmd__remote__subcmd__delete__subcmd__project"
                ;;
            geectl__subcmd__remote,devel-project)
                cmd="geectl__subcmd__remote__subcmd__devel__subcmd__project"
                ;;
            geectl__subcmd__remote,diff-packages)
                cmd="geectl__subcmd__remote__subcmd__diff__subcmd__packages"
                ;;
            geectl__subcmd__remote,download)
                cmd="geectl__subcmd__remote__subcmd__download"
                ;;
            geectl__subcmd__remote,help)
                cmd="geectl__subcmd__remote__subcmd__help"
                ;;
            geectl__subcmd__remote,link-target)
                cmd="geectl__subcmd__remote__subcmd__link__subcmd__target"
                ;;
            geectl__subcmd__remote,list-files)
                cmd="geectl__subcmd__remote__subcmd__list__subcmd__files"
                ;;
            geectl__subcmd__remote,list-packages)
                cmd="geectl__subcmd__remote__subcmd__list__subcmd__packages"
                ;;
            geectl__subcmd__remote,list-projects)
                cmd="geectl__subcmd__remote__subcmd__list__subcmd__projects"
                ;;
            geectl__subcmd__remote,list-scmsync-projects)
                cmd="geectl__subcmd__remote__subcmd__list__subcmd__scmsync__subcmd__projects"
                ;;
            geectl__subcmd__remote,list-staging-projects)
                cmd="geectl__subcmd__remote__subcmd__list__subcmd__staging__subcmd__projects"
                ;;
            geectl__subcmd__remote,lock)
                cmd="geectl__subcmd__remote__subcmd__lock"
                ;;
            geectl__subcmd__remote,maintainers)
                cmd="geectl__subcmd__remote__subcmd__maintainers"
                ;;
            geectl__subcmd__remote,owners)
                cmd="geectl__subcmd__remote__subcmd__owners"
                ;;
            geectl__subcmd__remote,owners-for-binary)
                cmd="geectl__subcmd__remote__subcmd__owners__subcmd__for__subcmd__binary"
                ;;
            geectl__subcmd__remote,owners-for-package)
                cmd="geectl__subcmd__remote__subcmd__owners__subcmd__for__subcmd__package"
                ;;
            geectl__subcmd__remote,package-log)
                cmd="geectl__subcmd__remote__subcmd__package__subcmd__log"
                ;;
            geectl__subcmd__remote,package-meta)
                cmd="geectl__subcmd__remote__subcmd__package__subcmd__meta"
                ;;
            geectl__subcmd__remote,project-diff)
                cmd="geectl__subcmd__remote__subcmd__project__subcmd__diff"
                ;;
            geectl__subcmd__remote,project-meta)
                cmd="geectl__subcmd__remote__subcmd__project__subcmd__meta"
                ;;
            geectl__subcmd__remote,release-package)
                cmd="geectl__subcmd__remote__subcmd__release__subcmd__package"
                ;;
            geectl__subcmd__remote,release-project)
                cmd="geectl__subcmd__remote__subcmd__release__subcmd__project"
                ;;
            geectl__subcmd__remote,repo-add)
                cmd="geectl__subcmd__remote__subcmd__repo__subcmd__add"
                ;;
            geectl__subcmd__remote,repo-remove)
                cmd="geectl__subcmd__remote__subcmd__repo__subcmd__remove"
                ;;
            geectl__subcmd__remote,revision-patch)
                cmd="geectl__subcmd__remote__subcmd__revision__subcmd__patch"
                ;;
            geectl__subcmd__remote,search-packages)
                cmd="geectl__subcmd__remote__subcmd__search__subcmd__packages"
                ;;
            geectl__subcmd__remote,search-packages-by-person)
                cmd="geectl__subcmd__remote__subcmd__search__subcmd__packages__subcmd__by__subcmd__person"
                ;;
            geectl__subcmd__remote,search-projects)
                cmd="geectl__subcmd__remote__subcmd__search__subcmd__projects"
                ;;
            geectl__subcmd__remote,search-projects-by-person)
                cmd="geectl__subcmd__remote__subcmd__search__subcmd__projects__subcmd__by__subcmd__person"
                ;;
            geectl__subcmd__remote,set-package-meta)
                cmd="geectl__subcmd__remote__subcmd__set__subcmd__package__subcmd__meta"
                ;;
            geectl__subcmd__remote,set-project-meta)
                cmd="geectl__subcmd__remote__subcmd__set__subcmd__project__subcmd__meta"
                ;;
            geectl__subcmd__remote,staging-project)
                cmd="geectl__subcmd__remote__subcmd__staging__subcmd__project"
                ;;
            geectl__subcmd__remote,undelete-package)
                cmd="geectl__subcmd__remote__subcmd__undelete__subcmd__package"
                ;;
            geectl__subcmd__remote,undelete-project)
                cmd="geectl__subcmd__remote__subcmd__undelete__subcmd__project"
                ;;
            geectl__subcmd__remote,unlock)
                cmd="geectl__subcmd__remote__subcmd__unlock"
                ;;
            geectl__subcmd__remote,update-package-meta-from-spec)
                cmd="geectl__subcmd__remote__subcmd__update__subcmd__package__subcmd__meta__subcmd__from__subcmd__spec"
                ;;
            geectl__subcmd__remote,upload-file)
                cmd="geectl__subcmd__remote__subcmd__upload__subcmd__file"
                ;;
            geectl__subcmd__remote,whois)
                cmd="geectl__subcmd__remote__subcmd__whois"
                ;;
            geectl__subcmd__remote__subcmd__help,add-channels)
                cmd="geectl__subcmd__remote__subcmd__help__subcmd__add__subcmd__channels"
                ;;
            geectl__subcmd__remote__subcmd__help,add-containers)
                cmd="geectl__subcmd__remote__subcmd__help__subcmd__add__subcmd__containers"
                ;;
            geectl__subcmd__remote__subcmd__help,aggregate-package)
                cmd="geectl__subcmd__remote__subcmd__help__subcmd__aggregate__subcmd__package"
                ;;
            geectl__subcmd__remote__subcmd__help,cat)
                cmd="geectl__subcmd__remote__subcmd__help__subcmd__cat"
                ;;
            geectl__subcmd__remote__subcmd__help,count-scmsync-projects)
                cmd="geectl__subcmd__remote__subcmd__help__subcmd__count__subcmd__scmsync__subcmd__projects"
                ;;
            geectl__subcmd__remote__subcmd__help,create-package)
                cmd="geectl__subcmd__remote__subcmd__help__subcmd__create__subcmd__package"
                ;;
            geectl__subcmd__remote__subcmd__help,create-project)
                cmd="geectl__subcmd__remote__subcmd__help__subcmd__create__subcmd__project"
                ;;
            geectl__subcmd__remote__subcmd__help,create-token)
                cmd="geectl__subcmd__remote__subcmd__help__subcmd__create__subcmd__token"
                ;;
            geectl__subcmd__remote__subcmd__help,delete-file)
                cmd="geectl__subcmd__remote__subcmd__help__subcmd__delete__subcmd__file"
                ;;
            geectl__subcmd__remote__subcmd__help,delete-link)
                cmd="geectl__subcmd__remote__subcmd__help__subcmd__delete__subcmd__link"
                ;;
            geectl__subcmd__remote__subcmd__help,delete-package)
                cmd="geectl__subcmd__remote__subcmd__help__subcmd__delete__subcmd__package"
                ;;
            geectl__subcmd__remote__subcmd__help,delete-project)
                cmd="geectl__subcmd__remote__subcmd__help__subcmd__delete__subcmd__project"
                ;;
            geectl__subcmd__remote__subcmd__help,devel-project)
                cmd="geectl__subcmd__remote__subcmd__help__subcmd__devel__subcmd__project"
                ;;
            geectl__subcmd__remote__subcmd__help,diff-packages)
                cmd="geectl__subcmd__remote__subcmd__help__subcmd__diff__subcmd__packages"
                ;;
            geectl__subcmd__remote__subcmd__help,download)
                cmd="geectl__subcmd__remote__subcmd__help__subcmd__download"
                ;;
            geectl__subcmd__remote__subcmd__help,help)
                cmd="geectl__subcmd__remote__subcmd__help__subcmd__help"
                ;;
            geectl__subcmd__remote__subcmd__help,link-target)
                cmd="geectl__subcmd__remote__subcmd__help__subcmd__link__subcmd__target"
                ;;
            geectl__subcmd__remote__subcmd__help,list-files)
                cmd="geectl__subcmd__remote__subcmd__help__subcmd__list__subcmd__files"
                ;;
            geectl__subcmd__remote__subcmd__help,list-packages)
                cmd="geectl__subcmd__remote__subcmd__help__subcmd__list__subcmd__packages"
                ;;
            geectl__subcmd__remote__subcmd__help,list-projects)
                cmd="geectl__subcmd__remote__subcmd__help__subcmd__list__subcmd__projects"
                ;;
            geectl__subcmd__remote__subcmd__help,list-scmsync-projects)
                cmd="geectl__subcmd__remote__subcmd__help__subcmd__list__subcmd__scmsync__subcmd__projects"
                ;;
            geectl__subcmd__remote__subcmd__help,list-staging-projects)
                cmd="geectl__subcmd__remote__subcmd__help__subcmd__list__subcmd__staging__subcmd__projects"
                ;;
            geectl__subcmd__remote__subcmd__help,lock)
                cmd="geectl__subcmd__remote__subcmd__help__subcmd__lock"
                ;;
            geectl__subcmd__remote__subcmd__help,maintainers)
                cmd="geectl__subcmd__remote__subcmd__help__subcmd__maintainers"
                ;;
            geectl__subcmd__remote__subcmd__help,owners)
                cmd="geectl__subcmd__remote__subcmd__help__subcmd__owners"
                ;;
            geectl__subcmd__remote__subcmd__help,owners-for-binary)
                cmd="geectl__subcmd__remote__subcmd__help__subcmd__owners__subcmd__for__subcmd__binary"
                ;;
            geectl__subcmd__remote__subcmd__help,owners-for-package)
                cmd="geectl__subcmd__remote__subcmd__help__subcmd__owners__subcmd__for__subcmd__package"
                ;;
            geectl__subcmd__remote__subcmd__help,package-log)
                cmd="geectl__subcmd__remote__subcmd__help__subcmd__package__subcmd__log"
                ;;
            geectl__subcmd__remote__subcmd__help,package-meta)
                cmd="geectl__subcmd__remote__subcmd__help__subcmd__package__subcmd__meta"
                ;;
            geectl__subcmd__remote__subcmd__help,project-diff)
                cmd="geectl__subcmd__remote__subcmd__help__subcmd__project__subcmd__diff"
                ;;
            geectl__subcmd__remote__subcmd__help,project-meta)
                cmd="geectl__subcmd__remote__subcmd__help__subcmd__project__subcmd__meta"
                ;;
            geectl__subcmd__remote__subcmd__help,release-package)
                cmd="geectl__subcmd__remote__subcmd__help__subcmd__release__subcmd__package"
                ;;
            geectl__subcmd__remote__subcmd__help,release-project)
                cmd="geectl__subcmd__remote__subcmd__help__subcmd__release__subcmd__project"
                ;;
            geectl__subcmd__remote__subcmd__help,repo-add)
                cmd="geectl__subcmd__remote__subcmd__help__subcmd__repo__subcmd__add"
                ;;
            geectl__subcmd__remote__subcmd__help,repo-remove)
                cmd="geectl__subcmd__remote__subcmd__help__subcmd__repo__subcmd__remove"
                ;;
            geectl__subcmd__remote__subcmd__help,revision-patch)
                cmd="geectl__subcmd__remote__subcmd__help__subcmd__revision__subcmd__patch"
                ;;
            geectl__subcmd__remote__subcmd__help,search-packages)
                cmd="geectl__subcmd__remote__subcmd__help__subcmd__search__subcmd__packages"
                ;;
            geectl__subcmd__remote__subcmd__help,search-packages-by-person)
                cmd="geectl__subcmd__remote__subcmd__help__subcmd__search__subcmd__packages__subcmd__by__subcmd__person"
                ;;
            geectl__subcmd__remote__subcmd__help,search-projects)
                cmd="geectl__subcmd__remote__subcmd__help__subcmd__search__subcmd__projects"
                ;;
            geectl__subcmd__remote__subcmd__help,search-projects-by-person)
                cmd="geectl__subcmd__remote__subcmd__help__subcmd__search__subcmd__projects__subcmd__by__subcmd__person"
                ;;
            geectl__subcmd__remote__subcmd__help,set-package-meta)
                cmd="geectl__subcmd__remote__subcmd__help__subcmd__set__subcmd__package__subcmd__meta"
                ;;
            geectl__subcmd__remote__subcmd__help,set-project-meta)
                cmd="geectl__subcmd__remote__subcmd__help__subcmd__set__subcmd__project__subcmd__meta"
                ;;
            geectl__subcmd__remote__subcmd__help,staging-project)
                cmd="geectl__subcmd__remote__subcmd__help__subcmd__staging__subcmd__project"
                ;;
            geectl__subcmd__remote__subcmd__help,undelete-package)
                cmd="geectl__subcmd__remote__subcmd__help__subcmd__undelete__subcmd__package"
                ;;
            geectl__subcmd__remote__subcmd__help,undelete-project)
                cmd="geectl__subcmd__remote__subcmd__help__subcmd__undelete__subcmd__project"
                ;;
            geectl__subcmd__remote__subcmd__help,unlock)
                cmd="geectl__subcmd__remote__subcmd__help__subcmd__unlock"
                ;;
            geectl__subcmd__remote__subcmd__help,update-package-meta-from-spec)
                cmd="geectl__subcmd__remote__subcmd__help__subcmd__update__subcmd__package__subcmd__meta__subcmd__from__subcmd__spec"
                ;;
            geectl__subcmd__remote__subcmd__help,upload-file)
                cmd="geectl__subcmd__remote__subcmd__help__subcmd__upload__subcmd__file"
                ;;
            geectl__subcmd__remote__subcmd__help,whois)
                cmd="geectl__subcmd__remote__subcmd__help__subcmd__whois"
                ;;
            geectl__subcmd__request,accept)
                cmd="geectl__subcmd__request__subcmd__accept"
                ;;
            geectl__subcmd__request,add-comment)
                cmd="geectl__subcmd__request__subcmd__add__subcmd__comment"
                ;;
            geectl__subcmd__request,add-review)
                cmd="geectl__subcmd__request__subcmd__add__subcmd__review"
                ;;
            geectl__subcmd__request,change-review-state)
                cmd="geectl__subcmd__request__subcmd__change__subcmd__review__subcmd__state"
                ;;
            geectl__subcmd__request,change-state)
                cmd="geectl__subcmd__request__subcmd__change__subcmd__state"
                ;;
            geectl__subcmd__request,decline)
                cmd="geectl__subcmd__request__subcmd__decline"
                ;;
            geectl__subcmd__request,diff)
                cmd="geectl__subcmd__request__subcmd__diff"
                ;;
            geectl__subcmd__request,file)
                cmd="geectl__subcmd__request__subcmd__file"
                ;;
            geectl__subcmd__request,file-add-role)
                cmd="geectl__subcmd__request__subcmd__file__subcmd__add__subcmd__role"
                ;;
            geectl__subcmd__request,file-change-devel)
                cmd="geectl__subcmd__request__subcmd__file__subcmd__change__subcmd__devel"
                ;;
            geectl__subcmd__request,file-delete)
                cmd="geectl__subcmd__request__subcmd__file__subcmd__delete"
                ;;
            geectl__subcmd__request,file-maintenance-incident)
                cmd="geectl__subcmd__request__subcmd__file__subcmd__maintenance__subcmd__incident"
                ;;
            geectl__subcmd__request,help)
                cmd="geectl__subcmd__request__subcmd__help"
                ;;
            geectl__subcmd__request,list)
                cmd="geectl__subcmd__request__subcmd__list"
                ;;
            geectl__subcmd__request,list-comments)
                cmd="geectl__subcmd__request__subcmd__list__subcmd__comments"
                ;;
            geectl__subcmd__request,log)
                cmd="geectl__subcmd__request__subcmd__log"
                ;;
            geectl__subcmd__request,refile)
                cmd="geectl__subcmd__request__subcmd__refile"
                ;;
            geectl__subcmd__request,refile-maintenance-incident)
                cmd="geectl__subcmd__request__subcmd__refile__subcmd__maintenance__subcmd__incident"
                ;;
            geectl__subcmd__request,revoke)
                cmd="geectl__subcmd__request__subcmd__revoke"
                ;;
            geectl__subcmd__request,search)
                cmd="geectl__subcmd__request__subcmd__search"
                ;;
            geectl__subcmd__request,show)
                cmd="geectl__subcmd__request__subcmd__show"
                ;;
            geectl__subcmd__request,supersede)
                cmd="geectl__subcmd__request__subcmd__supersede"
                ;;
            geectl__subcmd__request,try-change-state)
                cmd="geectl__subcmd__request__subcmd__try__subcmd__change__subcmd__state"
                ;;
            geectl__subcmd__request__subcmd__help,accept)
                cmd="geectl__subcmd__request__subcmd__help__subcmd__accept"
                ;;
            geectl__subcmd__request__subcmd__help,add-comment)
                cmd="geectl__subcmd__request__subcmd__help__subcmd__add__subcmd__comment"
                ;;
            geectl__subcmd__request__subcmd__help,add-review)
                cmd="geectl__subcmd__request__subcmd__help__subcmd__add__subcmd__review"
                ;;
            geectl__subcmd__request__subcmd__help,change-review-state)
                cmd="geectl__subcmd__request__subcmd__help__subcmd__change__subcmd__review__subcmd__state"
                ;;
            geectl__subcmd__request__subcmd__help,change-state)
                cmd="geectl__subcmd__request__subcmd__help__subcmd__change__subcmd__state"
                ;;
            geectl__subcmd__request__subcmd__help,decline)
                cmd="geectl__subcmd__request__subcmd__help__subcmd__decline"
                ;;
            geectl__subcmd__request__subcmd__help,diff)
                cmd="geectl__subcmd__request__subcmd__help__subcmd__diff"
                ;;
            geectl__subcmd__request__subcmd__help,file)
                cmd="geectl__subcmd__request__subcmd__help__subcmd__file"
                ;;
            geectl__subcmd__request__subcmd__help,file-add-role)
                cmd="geectl__subcmd__request__subcmd__help__subcmd__file__subcmd__add__subcmd__role"
                ;;
            geectl__subcmd__request__subcmd__help,file-change-devel)
                cmd="geectl__subcmd__request__subcmd__help__subcmd__file__subcmd__change__subcmd__devel"
                ;;
            geectl__subcmd__request__subcmd__help,file-delete)
                cmd="geectl__subcmd__request__subcmd__help__subcmd__file__subcmd__delete"
                ;;
            geectl__subcmd__request__subcmd__help,file-maintenance-incident)
                cmd="geectl__subcmd__request__subcmd__help__subcmd__file__subcmd__maintenance__subcmd__incident"
                ;;
            geectl__subcmd__request__subcmd__help,help)
                cmd="geectl__subcmd__request__subcmd__help__subcmd__help"
                ;;
            geectl__subcmd__request__subcmd__help,list)
                cmd="geectl__subcmd__request__subcmd__help__subcmd__list"
                ;;
            geectl__subcmd__request__subcmd__help,list-comments)
                cmd="geectl__subcmd__request__subcmd__help__subcmd__list__subcmd__comments"
                ;;
            geectl__subcmd__request__subcmd__help,log)
                cmd="geectl__subcmd__request__subcmd__help__subcmd__log"
                ;;
            geectl__subcmd__request__subcmd__help,refile)
                cmd="geectl__subcmd__request__subcmd__help__subcmd__refile"
                ;;
            geectl__subcmd__request__subcmd__help,refile-maintenance-incident)
                cmd="geectl__subcmd__request__subcmd__help__subcmd__refile__subcmd__maintenance__subcmd__incident"
                ;;
            geectl__subcmd__request__subcmd__help,revoke)
                cmd="geectl__subcmd__request__subcmd__help__subcmd__revoke"
                ;;
            geectl__subcmd__request__subcmd__help,search)
                cmd="geectl__subcmd__request__subcmd__help__subcmd__search"
                ;;
            geectl__subcmd__request__subcmd__help,show)
                cmd="geectl__subcmd__request__subcmd__help__subcmd__show"
                ;;
            geectl__subcmd__request__subcmd__help,supersede)
                cmd="geectl__subcmd__request__subcmd__help__subcmd__supersede"
                ;;
            geectl__subcmd__request__subcmd__help,try-change-state)
                cmd="geectl__subcmd__request__subcmd__help__subcmd__try__subcmd__change__subcmd__state"
                ;;
            geectl__subcmd__service,help)
                cmd="geectl__subcmd__service__subcmd__help"
                ;;
            geectl__subcmd__service,list)
                cmd="geectl__subcmd__service__subcmd__list"
                ;;
            geectl__subcmd__service,state)
                cmd="geectl__subcmd__service__subcmd__state"
                ;;
            geectl__subcmd__service,trigger)
                cmd="geectl__subcmd__service__subcmd__trigger"
                ;;
            geectl__subcmd__service__subcmd__help,help)
                cmd="geectl__subcmd__service__subcmd__help__subcmd__help"
                ;;
            geectl__subcmd__service__subcmd__help,list)
                cmd="geectl__subcmd__service__subcmd__help__subcmd__list"
                ;;
            geectl__subcmd__service__subcmd__help,state)
                cmd="geectl__subcmd__service__subcmd__help__subcmd__state"
                ;;
            geectl__subcmd__service__subcmd__help,trigger)
                cmd="geectl__subcmd__service__subcmd__help__subcmd__trigger"
                ;;
            *)
                ;;
        esac
    done

    case "${cmd}" in
        geectl)
            opts="-h -V --format --debug --debug-body --live --instance --help --version version config remote request branch build local-build maintenance service gitea pull-request person meta changelog checkout update status diff add rm addremove commit pull resolve prjconf help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 1 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__add)
            opts="-h --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__addremove)
            opts="-h --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__branch)
            opts="-h --format --debug --debug-body --live --instance --help create branch-package copy-package copy-project set-devel-project link-package repair-link set-link-rev set-link-rev-project link-to-branch detach-branch help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__branch__subcmd__branch__subcmd__package)
            opts="-h --project --package --target-project --target-package --no-devel-project --new-package --force --revision --linkrev --comment --noaccess --add-repositories --add-repositories-block --add-repositories-rebuild --extend-package-names --maintenance --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --project)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --package)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --target-project)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --target-package)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --revision)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --linkrev)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --comment)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --add-repositories-block)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --add-repositories-rebuild)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__branch__subcmd__copy__subcmd__package)
            opts="-h --source-project --source-package --target-project --target-package --revision --keep-link --expand --comment --noservice --dont-update-source --with-vrev --to-apiurl --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --source-project)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --source-package)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --target-project)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --target-package)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --revision)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --comment)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --to-apiurl)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__branch__subcmd__copy__subcmd__project)
            opts="-h --source-project --target-project --with-binaries --with-history --make-older --make-origin-older --noservice --resign --comment --no-delay --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --source-project)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --target-project)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --comment)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__branch__subcmd__create)
            opts="-h --project --package --new-repo-name --organization --git-branch --no-devel-project --target-project --target-package --first-build-deadline-secs --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --project)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --package)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --new-repo-name)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --organization)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --git-branch)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --target-project)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --target-package)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --first-build-deadline-secs)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__branch__subcmd__detach__subcmd__branch)
            opts="-h --project --package --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --project)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --package)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__branch__subcmd__help)
            opts="create branch-package copy-package copy-project set-devel-project link-package repair-link set-link-rev set-link-rev-project link-to-branch detach-branch help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__branch__subcmd__help__subcmd__branch__subcmd__package)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__branch__subcmd__help__subcmd__copy__subcmd__package)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__branch__subcmd__help__subcmd__copy__subcmd__project)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__branch__subcmd__help__subcmd__create)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__branch__subcmd__help__subcmd__detach__subcmd__branch)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__branch__subcmd__help__subcmd__help)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__branch__subcmd__help__subcmd__link__subcmd__package)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__branch__subcmd__help__subcmd__link__subcmd__to__subcmd__branch)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__branch__subcmd__help__subcmd__repair__subcmd__link)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__branch__subcmd__help__subcmd__set__subcmd__devel__subcmd__project)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__branch__subcmd__help__subcmd__set__subcmd__link__subcmd__rev)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__branch__subcmd__help__subcmd__set__subcmd__link__subcmd__rev__subcmd__project)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__branch__subcmd__link__subcmd__package)
            opts="-h --source-project --source-package --target-project --target-package --force --revision --vrev --cicount --missing-target --disable-build --disable-publish --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --source-project)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --source-package)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --target-project)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --target-package)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --revision)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --vrev)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --cicount)
                    COMPREPLY=($(compgen -W "add copy local" -- "${cur}"))
                    return 0
                    ;;
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__branch__subcmd__link__subcmd__to__subcmd__branch)
            opts="-h --project --package --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --project)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --package)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__branch__subcmd__repair__subcmd__link)
            opts="-h --project --package --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --project)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --package)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__branch__subcmd__set__subcmd__devel__subcmd__project)
            opts="-h --project --package --devel-project --devel-package --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --project)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --package)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --devel-project)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --devel-package)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__branch__subcmd__set__subcmd__link__subcmd__rev)
            opts="-h --project --package --revision --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --project)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --package)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --revision)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__branch__subcmd__set__subcmd__link__subcmd__rev__subcmd__project)
            opts="-h --project --revision --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --project)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --revision)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__build)
            opts="-h --format --debug --debug-body --live --instance --help results wait-for-results build-log build-status job-status rebuild-package restart-build abort-build wipe-binaries buildinfo buildconfig build-history job-history list-binaries download-binary fileinfo dependson whatdependson check-constraints list-repositories help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__build__subcmd__abort__subcmd__build)
            opts="-h --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__build__subcmd__build__subcmd__history)
            opts="-h --limit --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --limit)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__build__subcmd__build__subcmd__log)
            opts="-h --start --last --lastsucceeded --multibuild --strip-time --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --start)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --multibuild)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__build__subcmd__build__subcmd__status)
            opts="-h --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__build__subcmd__buildconfig)
            opts="-h --alternative-project --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --alternative-project)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__build__subcmd__buildinfo)
            opts="-h --alternative-project --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --alternative-project)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__build__subcmd__check__subcmd__constraints)
            opts="-h --constraints --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --constraints)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__build__subcmd__dependson)
            opts="-h --package --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --package)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__build__subcmd__download__subcmd__binary)
            opts="-h --output --all --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --output)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__build__subcmd__fileinfo)
            opts="-h --ext --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__build__subcmd__help)
            opts="results wait-for-results build-log build-status job-status rebuild-package restart-build abort-build wipe-binaries buildinfo buildconfig build-history job-history list-binaries download-binary fileinfo dependson whatdependson check-constraints list-repositories help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__build__subcmd__help__subcmd__abort__subcmd__build)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__build__subcmd__help__subcmd__build__subcmd__history)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__build__subcmd__help__subcmd__build__subcmd__log)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__build__subcmd__help__subcmd__build__subcmd__status)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__build__subcmd__help__subcmd__buildconfig)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__build__subcmd__help__subcmd__buildinfo)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__build__subcmd__help__subcmd__check__subcmd__constraints)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__build__subcmd__help__subcmd__dependson)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__build__subcmd__help__subcmd__download__subcmd__binary)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__build__subcmd__help__subcmd__fileinfo)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__build__subcmd__help__subcmd__help)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__build__subcmd__help__subcmd__job__subcmd__history)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__build__subcmd__help__subcmd__job__subcmd__status)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__build__subcmd__help__subcmd__list__subcmd__binaries)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__build__subcmd__help__subcmd__list__subcmd__repositories)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__build__subcmd__help__subcmd__rebuild__subcmd__package)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__build__subcmd__help__subcmd__restart__subcmd__build)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__build__subcmd__help__subcmd__results)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__build__subcmd__help__subcmd__wait__subcmd__for__subcmd__results)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__build__subcmd__help__subcmd__whatdependson)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__build__subcmd__help__subcmd__wipe__subcmd__binaries)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__build__subcmd__job__subcmd__history)
            opts="-h --package --limit --code --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --package)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --limit)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --code)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__build__subcmd__job__subcmd__status)
            opts="-h --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__build__subcmd__list__subcmd__binaries)
            opts="-h --package --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --package)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__build__subcmd__list__subcmd__repositories)
            opts="-h --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__build__subcmd__rebuild__subcmd__package)
            opts="-h --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__build__subcmd__restart__subcmd__build)
            opts="-h --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__build__subcmd__results)
            opts="-h --package --repository --arch --code --multibuild --locallink --lastbuild --oldstate --all-instances --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --package)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --repository)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --arch)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --code)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --oldstate)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__build__subcmd__wait__subcmd__for__subcmd__results)
            opts="-h --package --repository --arch --code --multibuild --locallink --lastbuild --deadline-secs --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --package)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --repository)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --arch)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --code)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline-secs)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__build__subcmd__whatdependson)
            opts="-h --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__build__subcmd__wipe__subcmd__binaries)
            opts="-h --arch --repository --all --build-disabled --build-failed --broken --unresolvable --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --arch)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --repository)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__changelog)
            opts="-h --format --debug --debug-body --live --instance --help validate generate identity help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__changelog__subcmd__generate)
            opts="-m -h --target-project --target-package --author --message --write --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --target-project)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --target-package)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --author)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --message)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                -m)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__changelog__subcmd__help)
            opts="validate generate identity help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__changelog__subcmd__help__subcmd__generate)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__changelog__subcmd__help__subcmd__help)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__changelog__subcmd__help__subcmd__identity)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__changelog__subcmd__help__subcmd__validate)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__changelog__subcmd__identity)
            opts="-h --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__changelog__subcmd__validate)
            opts="-h --target-project --target-package --entries --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --target-project)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --target-package)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --entries)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__checkout)
            opts="-h --rev --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --rev)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__commit)
            opts="-m -h --message --noservice --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --message)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                -m)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__config)
            opts="-h --format --debug --debug-body --live --instance --help show init import-oscrc help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__config__subcmd__help)
            opts="show init import-oscrc help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__config__subcmd__help__subcmd__help)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__config__subcmd__help__subcmd__import__subcmd__oscrc)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__config__subcmd__help__subcmd__init)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__config__subcmd__help__subcmd__show)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__config__subcmd__import__subcmd__oscrc)
            opts="-h --oscrc --apiurl --alias --force --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --oscrc)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --apiurl)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --alias)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__config__subcmd__init)
            opts="-h --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__config__subcmd__show)
            opts="-h --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__diff)
            opts="-h --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__gitea)
            opts="-h --format --debug --debug-body --live --instance --help repo-get repo-list repo-search whoami repo-create repo-delete repo-fork branch-list branch-get branch-head raw-file commit-statuses maintainers cleanup-forks help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__gitea__subcmd__branch__subcmd__get)
            opts="-h --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__gitea__subcmd__branch__subcmd__head)
            opts="-h --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__gitea__subcmd__branch__subcmd__list)
            opts="-h --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__gitea__subcmd__cleanup__subcmd__forks)
            opts="-h --user --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --user)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__gitea__subcmd__commit__subcmd__statuses)
            opts="-h --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__gitea__subcmd__help)
            opts="repo-get repo-list repo-search whoami repo-create repo-delete repo-fork branch-list branch-get branch-head raw-file commit-statuses maintainers cleanup-forks help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__gitea__subcmd__help__subcmd__branch__subcmd__get)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__gitea__subcmd__help__subcmd__branch__subcmd__head)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__gitea__subcmd__help__subcmd__branch__subcmd__list)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__gitea__subcmd__help__subcmd__cleanup__subcmd__forks)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__gitea__subcmd__help__subcmd__commit__subcmd__statuses)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__gitea__subcmd__help__subcmd__help)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__gitea__subcmd__help__subcmd__maintainers)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__gitea__subcmd__help__subcmd__raw__subcmd__file)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__gitea__subcmd__help__subcmd__repo__subcmd__create)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__gitea__subcmd__help__subcmd__repo__subcmd__delete)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__gitea__subcmd__help__subcmd__repo__subcmd__fork)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__gitea__subcmd__help__subcmd__repo__subcmd__get)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__gitea__subcmd__help__subcmd__repo__subcmd__list)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__gitea__subcmd__help__subcmd__repo__subcmd__search)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__gitea__subcmd__help__subcmd__whoami)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__gitea__subcmd__maintainers)
            opts="-h --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__gitea__subcmd__raw__subcmd__file)
            opts="-h --ref --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --ref)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__gitea__subcmd__repo__subcmd__create)
            opts="-h --private --description --default-branch --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --description)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --default-branch)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__gitea__subcmd__repo__subcmd__delete)
            opts="-h --force --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__gitea__subcmd__repo__subcmd__fork)
            opts="-h --new-name --organization --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --new-name)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --organization)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__gitea__subcmd__repo__subcmd__get)
            opts="-h --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__gitea__subcmd__repo__subcmd__list)
            opts="-h --owner --org --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --owner)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --org)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__gitea__subcmd__repo__subcmd__search)
            opts="-h --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__gitea__subcmd__whoami)
            opts="-h --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help)
            opts="version config remote request branch build local-build maintenance service gitea pull-request person meta changelog checkout update status diff add rm addremove commit pull resolve prjconf help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__add)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__addremove)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__branch)
            opts="create branch-package copy-package copy-project set-devel-project link-package repair-link set-link-rev set-link-rev-project link-to-branch detach-branch"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__branch__subcmd__branch__subcmd__package)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__branch__subcmd__copy__subcmd__package)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__branch__subcmd__copy__subcmd__project)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__branch__subcmd__create)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__branch__subcmd__detach__subcmd__branch)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__branch__subcmd__link__subcmd__package)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__branch__subcmd__link__subcmd__to__subcmd__branch)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__branch__subcmd__repair__subcmd__link)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__branch__subcmd__set__subcmd__devel__subcmd__project)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__branch__subcmd__set__subcmd__link__subcmd__rev)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__branch__subcmd__set__subcmd__link__subcmd__rev__subcmd__project)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__build)
            opts="results wait-for-results build-log build-status job-status rebuild-package restart-build abort-build wipe-binaries buildinfo buildconfig build-history job-history list-binaries download-binary fileinfo dependson whatdependson check-constraints list-repositories"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__build__subcmd__abort__subcmd__build)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__build__subcmd__build__subcmd__history)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__build__subcmd__build__subcmd__log)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__build__subcmd__build__subcmd__status)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__build__subcmd__buildconfig)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__build__subcmd__buildinfo)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__build__subcmd__check__subcmd__constraints)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__build__subcmd__dependson)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__build__subcmd__download__subcmd__binary)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__build__subcmd__fileinfo)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__build__subcmd__job__subcmd__history)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__build__subcmd__job__subcmd__status)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__build__subcmd__list__subcmd__binaries)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__build__subcmd__list__subcmd__repositories)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__build__subcmd__rebuild__subcmd__package)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__build__subcmd__restart__subcmd__build)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__build__subcmd__results)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__build__subcmd__wait__subcmd__for__subcmd__results)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__build__subcmd__whatdependson)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__build__subcmd__wipe__subcmd__binaries)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__changelog)
            opts="validate generate identity"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__changelog__subcmd__generate)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__changelog__subcmd__identity)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__changelog__subcmd__validate)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__checkout)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__commit)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__config)
            opts="show init import-oscrc"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__config__subcmd__import__subcmd__oscrc)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__config__subcmd__init)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__config__subcmd__show)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__diff)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__gitea)
            opts="repo-get repo-list repo-search whoami repo-create repo-delete repo-fork branch-list branch-get branch-head raw-file commit-statuses maintainers cleanup-forks"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__gitea__subcmd__branch__subcmd__get)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__gitea__subcmd__branch__subcmd__head)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__gitea__subcmd__branch__subcmd__list)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__gitea__subcmd__cleanup__subcmd__forks)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__gitea__subcmd__commit__subcmd__statuses)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__gitea__subcmd__maintainers)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__gitea__subcmd__raw__subcmd__file)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__gitea__subcmd__repo__subcmd__create)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__gitea__subcmd__repo__subcmd__delete)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__gitea__subcmd__repo__subcmd__fork)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__gitea__subcmd__repo__subcmd__get)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__gitea__subcmd__repo__subcmd__list)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__gitea__subcmd__repo__subcmd__search)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__gitea__subcmd__whoami)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__help)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__local__subcmd__build)
            opts="service-run precommit spec-cleaner build wipe prune periodic-install periodic-uninstall periodic-status chroot log"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__local__subcmd__build__subcmd__build)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__local__subcmd__build__subcmd__chroot)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__local__subcmd__build__subcmd__log)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__local__subcmd__build__subcmd__periodic__subcmd__install)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__local__subcmd__build__subcmd__periodic__subcmd__status)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__local__subcmd__build__subcmd__periodic__subcmd__uninstall)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__local__subcmd__build__subcmd__precommit)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__local__subcmd__build__subcmd__prune)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__local__subcmd__build__subcmd__service__subcmd__run)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__local__subcmd__build__subcmd__spec__subcmd__cleaner)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__local__subcmd__build__subcmd__wipe)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__maintenance)
            opts="maintained-instances maintenance-branch"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__maintenance__subcmd__maintained__subcmd__instances)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__maintenance__subcmd__maintenance__subcmd__branch)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__meta)
            opts="set"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__meta__subcmd__set)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__person)
            opts="search"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__person__subcmd__search)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__prjconf)
            opts="show validate clean"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__prjconf__subcmd__clean)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__prjconf__subcmd__show)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__prjconf__subcmd__validate)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__pull)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__pull__subcmd__request)
            opts="show diff search approve request-changes comment build-comment comments cancel-review-request merge decline withdraw reopen submit cleanup"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__pull__subcmd__request__subcmd__approve)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__pull__subcmd__request__subcmd__build__subcmd__comment)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__pull__subcmd__request__subcmd__cancel__subcmd__review__subcmd__request)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__pull__subcmd__request__subcmd__cleanup)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__pull__subcmd__request__subcmd__comment)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__pull__subcmd__request__subcmd__comments)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__pull__subcmd__request__subcmd__decline)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__pull__subcmd__request__subcmd__diff)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__pull__subcmd__request__subcmd__merge)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__pull__subcmd__request__subcmd__reopen)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__pull__subcmd__request__subcmd__request__subcmd__changes)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__pull__subcmd__request__subcmd__search)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__pull__subcmd__request__subcmd__show)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__pull__subcmd__request__subcmd__submit)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__pull__subcmd__request__subcmd__withdraw)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__remote)
            opts="cat download list-files list-packages list-projects list-staging-projects staging-project package-log revision-patch project-meta set-project-meta package-meta set-package-meta update-package-meta-from-spec search-packages search-projects search-projects-by-person search-packages-by-person count-scmsync-projects list-scmsync-projects owners owners-for-binary owners-for-package maintainers whois link-target project-diff diff-packages devel-project repo-add repo-remove upload-file create-project create-package create-token delete-package delete-link delete-file delete-project undelete-package undelete-project aggregate-package release-package release-project lock unlock add-channels add-containers"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__remote__subcmd__add__subcmd__channels)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__remote__subcmd__add__subcmd__containers)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__remote__subcmd__aggregate__subcmd__package)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__remote__subcmd__cat)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__remote__subcmd__count__subcmd__scmsync__subcmd__projects)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__remote__subcmd__create__subcmd__package)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__remote__subcmd__create__subcmd__project)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__remote__subcmd__create__subcmd__token)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__remote__subcmd__delete__subcmd__file)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__remote__subcmd__delete__subcmd__link)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__remote__subcmd__delete__subcmd__package)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__remote__subcmd__delete__subcmd__project)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__remote__subcmd__devel__subcmd__project)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__remote__subcmd__diff__subcmd__packages)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__remote__subcmd__download)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__remote__subcmd__link__subcmd__target)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__remote__subcmd__list__subcmd__files)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__remote__subcmd__list__subcmd__packages)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__remote__subcmd__list__subcmd__projects)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__remote__subcmd__list__subcmd__scmsync__subcmd__projects)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__remote__subcmd__list__subcmd__staging__subcmd__projects)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__remote__subcmd__lock)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__remote__subcmd__maintainers)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__remote__subcmd__owners)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__remote__subcmd__owners__subcmd__for__subcmd__binary)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__remote__subcmd__owners__subcmd__for__subcmd__package)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__remote__subcmd__package__subcmd__log)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__remote__subcmd__package__subcmd__meta)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__remote__subcmd__project__subcmd__diff)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__remote__subcmd__project__subcmd__meta)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__remote__subcmd__release__subcmd__package)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__remote__subcmd__release__subcmd__project)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__remote__subcmd__repo__subcmd__add)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__remote__subcmd__repo__subcmd__remove)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__remote__subcmd__revision__subcmd__patch)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__remote__subcmd__search__subcmd__packages)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__remote__subcmd__search__subcmd__packages__subcmd__by__subcmd__person)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__remote__subcmd__search__subcmd__projects)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__remote__subcmd__search__subcmd__projects__subcmd__by__subcmd__person)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__remote__subcmd__set__subcmd__package__subcmd__meta)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__remote__subcmd__set__subcmd__project__subcmd__meta)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__remote__subcmd__staging__subcmd__project)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__remote__subcmd__undelete__subcmd__package)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__remote__subcmd__undelete__subcmd__project)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__remote__subcmd__unlock)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__remote__subcmd__update__subcmd__package__subcmd__meta__subcmd__from__subcmd__spec)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__remote__subcmd__upload__subcmd__file)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__remote__subcmd__whois)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__request)
            opts="show log search list diff list-comments accept decline revoke change-state try-change-state add-review change-review-state add-comment file refile supersede file-add-role file-change-devel file-delete file-maintenance-incident refile-maintenance-incident"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__request__subcmd__accept)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__request__subcmd__add__subcmd__comment)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__request__subcmd__add__subcmd__review)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__request__subcmd__change__subcmd__review__subcmd__state)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__request__subcmd__change__subcmd__state)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__request__subcmd__decline)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__request__subcmd__diff)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__request__subcmd__file)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__request__subcmd__file__subcmd__add__subcmd__role)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__request__subcmd__file__subcmd__change__subcmd__devel)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__request__subcmd__file__subcmd__delete)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__request__subcmd__file__subcmd__maintenance__subcmd__incident)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__request__subcmd__list)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__request__subcmd__list__subcmd__comments)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__request__subcmd__log)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__request__subcmd__refile)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__request__subcmd__refile__subcmd__maintenance__subcmd__incident)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__request__subcmd__revoke)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__request__subcmd__search)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__request__subcmd__show)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__request__subcmd__supersede)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__request__subcmd__try__subcmd__change__subcmd__state)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__resolve)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__rm)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__service)
            opts="list state trigger"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__service__subcmd__list)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__service__subcmd__state)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__service__subcmd__trigger)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__status)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__update)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__help__subcmd__version)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__local__subcmd__build)
            opts="-h --format --debug --debug-body --live --instance --help service-run precommit spec-cleaner build wipe prune periodic-install periodic-uninstall periodic-status chroot log help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__local__subcmd__build__subcmd__build)
            opts="-k -p -h --project --repo --arch --flavor --clean --release --trust-all-projects --refresh-image --alternative-project --root --noinit --preinstall-image --keep-rpms --prefer-packages --timeout-secs --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --project)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --repo)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --arch)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --flavor)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --release)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --alternative-project)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --root)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --keep-rpms)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                -k)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --prefer-packages)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                -p)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --timeout-secs)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__local__subcmd__build__subcmd__chroot)
            opts="-h --project --repo --arch --flavor --timeout-secs --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --project)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --repo)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --arch)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --flavor)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --timeout-secs)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__local__subcmd__build__subcmd__help)
            opts="service-run precommit spec-cleaner build wipe prune periodic-install periodic-uninstall periodic-status chroot log help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__local__subcmd__build__subcmd__help__subcmd__build)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__local__subcmd__build__subcmd__help__subcmd__chroot)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__local__subcmd__build__subcmd__help__subcmd__help)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__local__subcmd__build__subcmd__help__subcmd__log)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__local__subcmd__build__subcmd__help__subcmd__periodic__subcmd__install)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__local__subcmd__build__subcmd__help__subcmd__periodic__subcmd__status)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__local__subcmd__build__subcmd__help__subcmd__periodic__subcmd__uninstall)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__local__subcmd__build__subcmd__help__subcmd__precommit)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__local__subcmd__build__subcmd__help__subcmd__prune)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__local__subcmd__build__subcmd__help__subcmd__service__subcmd__run)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__local__subcmd__build__subcmd__help__subcmd__spec__subcmd__cleaner)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__local__subcmd__build__subcmd__help__subcmd__wipe)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__local__subcmd__build__subcmd__log)
            opts="-h --project --flavor --tail --full --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --project)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --flavor)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --tail)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__local__subcmd__build__subcmd__periodic__subcmd__install)
            opts="-h --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__local__subcmd__build__subcmd__periodic__subcmd__status)
            opts="-h --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__local__subcmd__build__subcmd__periodic__subcmd__uninstall)
            opts="-h --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__local__subcmd__build__subcmd__precommit)
            opts="-h --project --timeout-secs --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --project)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --timeout-secs)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__local__subcmd__build__subcmd__prune)
            opts="-h --apply --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__local__subcmd__build__subcmd__service__subcmd__run)
            opts="-h --mode --service --project --timeout-secs --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --mode)
                    COMPREPLY=($(compgen -W "run manualrun precommit localrun disabledrun" -- "${cur}"))
                    return 0
                    ;;
                --service)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --project)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --timeout-secs)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__local__subcmd__build__subcmd__spec__subcmd__cleaner)
            opts="-h --timeout-secs --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --timeout-secs)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__local__subcmd__build__subcmd__wipe)
            opts="-h --project --flavor --root --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --project)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --flavor)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --root)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__maintenance)
            opts="-h --format --debug --debug-body --live --instance --help maintained-instances maintenance-branch help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__maintenance__subcmd__help)
            opts="maintained-instances maintenance-branch help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__maintenance__subcmd__help__subcmd__help)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__maintenance__subcmd__help__subcmd__maintained__subcmd__instances)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__maintenance__subcmd__help__subcmd__maintenance__subcmd__branch)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__maintenance__subcmd__maintained__subcmd__instances)
            opts="-h --package --attribute --update-project-attribute --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --package)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --attribute)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --update-project-attribute)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__maintenance__subcmd__maintenance__subcmd__branch)
            opts="-h --package --attribute --update-project-attribute --target-project --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --package)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --attribute)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --update-project-attribute)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --target-project)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__meta)
            opts="-h --format --debug --debug-body --live --instance --help set help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__meta__subcmd__help)
            opts="set help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__meta__subcmd__help__subcmd__help)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__meta__subcmd__help__subcmd__set)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__meta__subcmd__set)
            opts="-h --project --package --repo --description --url --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --project)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --package)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --repo)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --description)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --url)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__person)
            opts="-h --format --debug --debug-body --live --instance --help search help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__person__subcmd__help)
            opts="search help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__person__subcmd__help__subcmd__help)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__person__subcmd__help__subcmd__search)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__person__subcmd__search)
            opts="-h --backend --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --backend)
                    COMPREPLY=($(compgen -W "obs gitea" -- "${cur}"))
                    return 0
                    ;;
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__prjconf)
            opts="-h --format --debug --debug-body --live --instance --help show validate clean help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__prjconf__subcmd__clean)
            opts="-h --include-path-dependent --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__prjconf__subcmd__help)
            opts="show validate clean help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__prjconf__subcmd__help__subcmd__clean)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__prjconf__subcmd__help__subcmd__help)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__prjconf__subcmd__help__subcmd__show)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__prjconf__subcmd__help__subcmd__validate)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__prjconf__subcmd__show)
            opts="-h --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__prjconf__subcmd__validate)
            opts="-h --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__pull)
            opts="-h --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__pull__subcmd__request)
            opts="-h --format --debug --debug-body --live --instance --help show diff search approve request-changes comment build-comment comments cancel-review-request merge decline withdraw reopen submit cleanup help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__pull__subcmd__request__subcmd__approve)
            opts="-h --message --commit --user --group --project --package --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --message)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --commit)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --user)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --group)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --project)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --package)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__pull__subcmd__request__subcmd__build__subcmd__comment)
            opts="-h --project --package --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --project)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --package)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__pull__subcmd__request__subcmd__cancel__subcmd__review__subcmd__request)
            opts="-h --reviewer --all --exclude --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --reviewer)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --exclude)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__pull__subcmd__request__subcmd__cleanup)
            opts="-h --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__pull__subcmd__request__subcmd__comment)
            opts="-h --body --parent --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --body)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --parent)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__pull__subcmd__request__subcmd__comments)
            opts="-h --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__pull__subcmd__request__subcmd__decline)
            opts="-h --message --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --message)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__pull__subcmd__request__subcmd__diff)
            opts="-h --xml --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__pull__subcmd__request__subcmd__help)
            opts="show diff search approve request-changes comment build-comment comments cancel-review-request merge decline withdraw reopen submit cleanup help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__pull__subcmd__request__subcmd__help__subcmd__approve)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__pull__subcmd__request__subcmd__help__subcmd__build__subcmd__comment)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__pull__subcmd__request__subcmd__help__subcmd__cancel__subcmd__review__subcmd__request)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__pull__subcmd__request__subcmd__help__subcmd__cleanup)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__pull__subcmd__request__subcmd__help__subcmd__comment)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__pull__subcmd__request__subcmd__help__subcmd__comments)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__pull__subcmd__request__subcmd__help__subcmd__decline)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__pull__subcmd__request__subcmd__help__subcmd__diff)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__pull__subcmd__request__subcmd__help__subcmd__help)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__pull__subcmd__request__subcmd__help__subcmd__merge)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__pull__subcmd__request__subcmd__help__subcmd__reopen)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__pull__subcmd__request__subcmd__help__subcmd__request__subcmd__changes)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__pull__subcmd__request__subcmd__help__subcmd__search)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__pull__subcmd__request__subcmd__help__subcmd__show)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__pull__subcmd__request__subcmd__help__subcmd__submit)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__pull__subcmd__request__subcmd__help__subcmd__withdraw)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__pull__subcmd__request__subcmd__merge)
            opts="-m -h --message --forward-message --method --force --no-forward --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --message)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --forward-message)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                -m)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --method)
                    COMPREPLY=($(compgen -W "merge rebase rebase-merge squash fast-forward-only" -- "${cur}"))
                    return 0
                    ;;
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__pull__subcmd__request__subcmd__reopen)
            opts="-h --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__pull__subcmd__request__subcmd__request__subcmd__changes)
            opts="-h --message --commit --user --group --project --package --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --message)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --commit)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --user)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --group)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --project)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --package)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__pull__subcmd__request__subcmd__search)
            opts="-h --backend --state --creator --reviewer --repo --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --backend)
                    COMPREPLY=($(compgen -W "obs gitea" -- "${cur}"))
                    return 0
                    ;;
                --state)
                    COMPREPLY=($(compgen -W "open merged declined withdrawn closed superseded deleted" -- "${cur}"))
                    return 0
                    ;;
                --creator)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --reviewer)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --repo)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__pull__subcmd__request__subcmd__show)
            opts="-h --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__pull__subcmd__request__subcmd__submit)
            opts="-h --target-project --target-package --description --changes-entry --cve --without-cleanup --repo --head --base --title --body --draft --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --target-project)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --target-package)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --description)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --changes-entry)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --cve)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --repo)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --head)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --base)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --title)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --body)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__pull__subcmd__request__subcmd__withdraw)
            opts="-h --message --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --message)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__remote)
            opts="-h --format --debug --debug-body --live --instance --help cat download list-files list-packages list-projects list-staging-projects staging-project package-log revision-patch project-meta set-project-meta package-meta set-package-meta update-package-meta-from-spec search-packages search-projects search-projects-by-person search-packages-by-person count-scmsync-projects list-scmsync-projects owners owners-for-binary owners-for-package maintainers whois link-target project-diff diff-packages devel-project repo-add repo-remove upload-file create-project create-package create-token delete-package delete-link delete-file delete-project undelete-package undelete-project aggregate-package release-package release-project lock unlock add-channels add-containers help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__remote__subcmd__add__subcmd__channels)
            opts="-h --enable-all --skip-disabled --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__remote__subcmd__add__subcmd__containers)
            opts="-h --extend-package-names --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__remote__subcmd__aggregate__subcmd__package)
            opts="-h --repository-map --no-sources --disable-publish --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --repository-map)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__remote__subcmd__cat)
            opts="-h --rev --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --rev)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__remote__subcmd__count__subcmd__scmsync__subcmd__projects)
            opts="-h --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__remote__subcmd__create__subcmd__package)
            opts="-h --title --description --scmsync --force --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --title)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --description)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --scmsync)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__remote__subcmd__create__subcmd__project)
            opts="-h --title --description --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --title)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --description)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__remote__subcmd__create__subcmd__token)
            opts="-h --operation --project --package --description --scm-token --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --operation)
                    COMPREPLY=($(compgen -W "runservice rebuild release workflow" -- "${cur}"))
                    return 0
                    ;;
                --project)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --package)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --description)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --scm-token)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__remote__subcmd__delete__subcmd__file)
            opts="-h --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__remote__subcmd__delete__subcmd__link)
            opts="-h --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__remote__subcmd__delete__subcmd__package)
            opts="-h --force --message --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --message)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__remote__subcmd__delete__subcmd__project)
            opts="-h --recursive --force --comment --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --comment)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__remote__subcmd__devel__subcmd__project)
            opts="-h --target-project --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --target-project)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__remote__subcmd__diff__subcmd__packages)
            opts="-h --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__remote__subcmd__download)
            opts="-h --rev --output --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --rev)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --output)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__remote__subcmd__help)
            opts="cat download list-files list-packages list-projects list-staging-projects staging-project package-log revision-patch project-meta set-project-meta package-meta set-package-meta update-package-meta-from-spec search-packages search-projects search-projects-by-person search-packages-by-person count-scmsync-projects list-scmsync-projects owners owners-for-binary owners-for-package maintainers whois link-target project-diff diff-packages devel-project repo-add repo-remove upload-file create-project create-package create-token delete-package delete-link delete-file delete-project undelete-package undelete-project aggregate-package release-package release-project lock unlock add-channels add-containers help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__remote__subcmd__help__subcmd__add__subcmd__channels)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__remote__subcmd__help__subcmd__add__subcmd__containers)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__remote__subcmd__help__subcmd__aggregate__subcmd__package)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__remote__subcmd__help__subcmd__cat)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__remote__subcmd__help__subcmd__count__subcmd__scmsync__subcmd__projects)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__remote__subcmd__help__subcmd__create__subcmd__package)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__remote__subcmd__help__subcmd__create__subcmd__project)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__remote__subcmd__help__subcmd__create__subcmd__token)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__remote__subcmd__help__subcmd__delete__subcmd__file)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__remote__subcmd__help__subcmd__delete__subcmd__link)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__remote__subcmd__help__subcmd__delete__subcmd__package)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__remote__subcmd__help__subcmd__delete__subcmd__project)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__remote__subcmd__help__subcmd__devel__subcmd__project)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__remote__subcmd__help__subcmd__diff__subcmd__packages)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__remote__subcmd__help__subcmd__download)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__remote__subcmd__help__subcmd__help)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__remote__subcmd__help__subcmd__link__subcmd__target)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__remote__subcmd__help__subcmd__list__subcmd__files)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__remote__subcmd__help__subcmd__list__subcmd__packages)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__remote__subcmd__help__subcmd__list__subcmd__projects)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__remote__subcmd__help__subcmd__list__subcmd__scmsync__subcmd__projects)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__remote__subcmd__help__subcmd__list__subcmd__staging__subcmd__projects)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__remote__subcmd__help__subcmd__lock)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__remote__subcmd__help__subcmd__maintainers)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__remote__subcmd__help__subcmd__owners)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__remote__subcmd__help__subcmd__owners__subcmd__for__subcmd__binary)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__remote__subcmd__help__subcmd__owners__subcmd__for__subcmd__package)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__remote__subcmd__help__subcmd__package__subcmd__log)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__remote__subcmd__help__subcmd__package__subcmd__meta)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__remote__subcmd__help__subcmd__project__subcmd__diff)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__remote__subcmd__help__subcmd__project__subcmd__meta)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__remote__subcmd__help__subcmd__release__subcmd__package)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__remote__subcmd__help__subcmd__release__subcmd__project)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__remote__subcmd__help__subcmd__repo__subcmd__add)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__remote__subcmd__help__subcmd__repo__subcmd__remove)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__remote__subcmd__help__subcmd__revision__subcmd__patch)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__remote__subcmd__help__subcmd__search__subcmd__packages)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__remote__subcmd__help__subcmd__search__subcmd__packages__subcmd__by__subcmd__person)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__remote__subcmd__help__subcmd__search__subcmd__projects)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__remote__subcmd__help__subcmd__search__subcmd__projects__subcmd__by__subcmd__person)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__remote__subcmd__help__subcmd__set__subcmd__package__subcmd__meta)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__remote__subcmd__help__subcmd__set__subcmd__project__subcmd__meta)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__remote__subcmd__help__subcmd__staging__subcmd__project)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__remote__subcmd__help__subcmd__undelete__subcmd__package)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__remote__subcmd__help__subcmd__undelete__subcmd__project)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__remote__subcmd__help__subcmd__unlock)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__remote__subcmd__help__subcmd__update__subcmd__package__subcmd__meta__subcmd__from__subcmd__spec)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__remote__subcmd__help__subcmd__upload__subcmd__file)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__remote__subcmd__help__subcmd__whois)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__remote__subcmd__link__subcmd__target)
            opts="-h --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__remote__subcmd__list__subcmd__files)
            opts="-h --rev --include-deleted --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --rev)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__remote__subcmd__list__subcmd__packages)
            opts="-h --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__remote__subcmd__list__subcmd__projects)
            opts="-h --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__remote__subcmd__list__subcmd__scmsync__subcmd__projects)
            opts="-h --limit --offset --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --limit)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --offset)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__remote__subcmd__list__subcmd__staging__subcmd__projects)
            opts="-h --no-requests --no-status --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__remote__subcmd__lock)
            opts="-h --message --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --message)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__remote__subcmd__maintainers)
            opts="-h --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__remote__subcmd__owners)
            opts="-h --verbose --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__remote__subcmd__owners__subcmd__for__subcmd__binary)
            opts="-h --verbose --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__remote__subcmd__owners__subcmd__for__subcmd__package)
            opts="-h --verbose --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__remote__subcmd__package__subcmd__log)
            opts="-h --range --limit --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --range)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --limit)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__remote__subcmd__package__subcmd__meta)
            opts="-h --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__remote__subcmd__project__subcmd__diff)
            opts="-h --include --exclude --annotate-requests --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --include)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --exclude)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__remote__subcmd__project__subcmd__meta)
            opts="-h --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__remote__subcmd__release__subcmd__package)
            opts="-h --repository --arch --target-project --target-repository --set-release --no-delay --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --repository)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --arch)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --target-project)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --target-repository)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --set-release)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__remote__subcmd__release__subcmd__project)
            opts="-h --repository --arch --target-project --target-repository --set-release --no-delay --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --repository)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --arch)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --target-project)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --target-repository)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --set-release)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__remote__subcmd__repo__subcmd__add)
            opts="-h --arch --path --disable-publish --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --arch)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --path)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__remote__subcmd__repo__subcmd__remove)
            opts="-h --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__remote__subcmd__revision__subcmd__patch)
            opts="-h --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__remote__subcmd__search__subcmd__packages)
            opts="-h --exact --devel-project --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --devel-project)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__remote__subcmd__search__subcmd__packages__subcmd__by__subcmd__person)
            opts="-h --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__remote__subcmd__search__subcmd__projects)
            opts="-h --exact --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__remote__subcmd__search__subcmd__projects__subcmd__by__subcmd__person)
            opts="-h --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__remote__subcmd__set__subcmd__package__subcmd__meta)
            opts="-h --from-file --title --description --url --scmsync --releasename --devel-project --devel-package --add-person --remove-person --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --from-file)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --title)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --description)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --url)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --scmsync)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --releasename)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --devel-project)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --devel-package)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --add-person)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --remove-person)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__remote__subcmd__set__subcmd__project__subcmd__meta)
            opts="-h --from-file --title --description --url --scmsync --devel-project --add-person --remove-person --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --from-file)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --title)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --description)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --url)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --scmsync)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --devel-project)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --add-person)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --remove-person)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__remote__subcmd__staging__subcmd__project)
            opts="-h --no-requests --no-status --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__remote__subcmd__undelete__subcmd__package)
            opts="-h --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__remote__subcmd__undelete__subcmd__project)
            opts="-h --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__remote__subcmd__unlock)
            opts="-h --message --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --message)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__remote__subcmd__update__subcmd__package__subcmd__meta__subcmd__from__subcmd__spec)
            opts="-h --spec --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --spec)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__remote__subcmd__upload__subcmd__file)
            opts="-h --from --binary --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --from)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__remote__subcmd__whois)
            opts="-h --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__request)
            opts="-h --format --debug --debug-body --live --instance --help show log search list diff list-comments accept decline revoke change-state try-change-state add-review change-review-state add-comment file refile supersede file-add-role file-change-devel file-delete file-maintenance-incident refile-maintenance-incident help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__request__subcmd__accept)
            opts="-m -h --comment --force --forward-message --no-forward --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --comment)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --forward-message)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                -m)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__request__subcmd__add__subcmd__comment)
            opts="-h --body --parent --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --body)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --parent)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__request__subcmd__add__subcmd__review)
            opts="-h --user --group --project --package --comment --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --user)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --group)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --project)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --package)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --comment)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__request__subcmd__change__subcmd__review__subcmd__state)
            opts="-h --user --group --project --package --comment --format --debug --debug-body --live --instance --help new accepted declined"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --user)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --group)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --project)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --package)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --comment)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__request__subcmd__change__subcmd__state)
            opts="-h --comment --force --format --debug --debug-body --live --instance --help new review accepted declined revoked"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --comment)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__request__subcmd__decline)
            opts="-h --comment --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --comment)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__request__subcmd__diff)
            opts="-h --xml --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__request__subcmd__file)
            opts="-h --source-project --source-package --target-project --target-package --description --changes-entry --cve --without-cleanup --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --source-project)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --source-package)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --target-project)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --target-package)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --description)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --changes-entry)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --cve)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__request__subcmd__file__subcmd__add__subcmd__role)
            opts="-h --target-project --target-package --person --person-role --group --group-role --description --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --target-project)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --target-package)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --person)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --person-role)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --group)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --group-role)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --description)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__request__subcmd__file__subcmd__change__subcmd__devel)
            opts="-h --source-project --source-package --target-project --target-package --description --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --source-project)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --source-package)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --target-project)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --target-package)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --description)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__request__subcmd__file__subcmd__delete)
            opts="-h --target-project --package --repository --whole-project --description --accept-in-hours --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --target-project)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --package)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --repository)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --description)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --accept-in-hours)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__request__subcmd__file__subcmd__maintenance__subcmd__incident)
            opts="-h --source-project --source-package --target-project --release-project --description --with-cleanup --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --source-project)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --source-package)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --target-project)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --release-project)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --description)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__request__subcmd__help)
            opts="show log search list diff list-comments accept decline revoke change-state try-change-state add-review change-review-state add-comment file refile supersede file-add-role file-change-devel file-delete file-maintenance-incident refile-maintenance-incident help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__request__subcmd__help__subcmd__accept)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__request__subcmd__help__subcmd__add__subcmd__comment)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__request__subcmd__help__subcmd__add__subcmd__review)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__request__subcmd__help__subcmd__change__subcmd__review__subcmd__state)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__request__subcmd__help__subcmd__change__subcmd__state)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__request__subcmd__help__subcmd__decline)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__request__subcmd__help__subcmd__diff)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__request__subcmd__help__subcmd__file)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__request__subcmd__help__subcmd__file__subcmd__add__subcmd__role)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__request__subcmd__help__subcmd__file__subcmd__change__subcmd__devel)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__request__subcmd__help__subcmd__file__subcmd__delete)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__request__subcmd__help__subcmd__file__subcmd__maintenance__subcmd__incident)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__request__subcmd__help__subcmd__help)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__request__subcmd__help__subcmd__list)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__request__subcmd__help__subcmd__list__subcmd__comments)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__request__subcmd__help__subcmd__log)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__request__subcmd__help__subcmd__refile)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__request__subcmd__help__subcmd__refile__subcmd__maintenance__subcmd__incident)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__request__subcmd__help__subcmd__revoke)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__request__subcmd__help__subcmd__search)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__request__subcmd__help__subcmd__show)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__request__subcmd__help__subcmd__supersede)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__request__subcmd__help__subcmd__try__subcmd__change__subcmd__state)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__request__subcmd__list)
            opts="-h --project --package --state --since --limit --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --project)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --package)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --state)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --since)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --limit)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__request__subcmd__list__subcmd__comments)
            opts="-h --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__request__subcmd__log)
            opts="-h --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__request__subcmd__refile)
            opts="-h --source-project --source-package --target-project --target-package --description --changes-entry --cve --without-cleanup --supersede --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --source-project)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --source-package)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --target-project)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --target-package)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --description)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --changes-entry)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --cve)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --supersede)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__request__subcmd__refile__subcmd__maintenance__subcmd__incident)
            opts="-h --source-project --source-package --target-project --release-project --description --with-cleanup --supersede --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --source-project)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --source-package)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --target-project)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --release-project)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --description)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --supersede)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__request__subcmd__revoke)
            opts="-h --comment --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --comment)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__request__subcmd__search)
            opts="-h --request-id --project --package --source-project --source-package --target-project --target-package --creator --who --reviewer --state --type --exclude-target-project --with-history --with-full-history --since --limit --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --request-id)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --project)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --package)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --source-project)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --source-package)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --target-project)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --target-package)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --creator)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --who)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --reviewer)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --state)
                    COMPREPLY=($(compgen -W "new review accepted declined revoked superseded deleted" -- "${cur}"))
                    return 0
                    ;;
                --type)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --exclude-target-project)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --since)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --limit)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__request__subcmd__show)
            opts="-h --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__request__subcmd__supersede)
            opts="-h --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__request__subcmd__try__subcmd__change__subcmd__state)
            opts="-h --comment --force --format --debug --debug-body --live --instance --help new review accepted declined revoked"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --comment)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__resolve)
            opts="-h --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__rm)
            opts="-h --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__service)
            opts="-h --format --debug --debug-body --live --instance --help list state trigger help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__service__subcmd__help)
            opts="list state trigger help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__service__subcmd__help__subcmd__help)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__service__subcmd__help__subcmd__list)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__service__subcmd__help__subcmd__state)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__service__subcmd__help__subcmd__trigger)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__service__subcmd__list)
            opts="-h --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__service__subcmd__state)
            opts="-h --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__service__subcmd__trigger)
            opts="-h --service --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --service)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__status)
            opts="-h --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__update)
            opts="-h --rev --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --rev)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        geectl__subcmd__version)
            opts="-h --format --debug --debug-body --live --instance --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --format)
                    COMPREPLY=($(compgen -W "json text" -- "${cur}"))
                    return 0
                    ;;
                --instance)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
    esac
}

if [[ "${BASH_VERSINFO[0]}" -eq 4 && "${BASH_VERSINFO[1]}" -ge 4 || "${BASH_VERSINFO[0]}" -gt 4 ]]; then
    complete -F _geectl -o nosort -o bashdefault -o default geectl
else
    complete -F _geectl -o bashdefault -o default geectl
fi
