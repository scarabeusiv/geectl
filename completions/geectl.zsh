#compdef geectl

autoload -U is-at-least

_geectl() {
    typeset -A opt_args
    typeset -a _arguments_options
    local ret=1

    if is-at-least 5.2; then
        _arguments_options=(-s -S -C)
    else
        _arguments_options=(-s -C)
    fi

    local context curcontext="$curcontext" state line
    _arguments "${_arguments_options[@]}" : \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
'-V[Print version]' \
'--version[Print version]' \
":: :_geectl_commands" \
"*::: :->geectl" \
&& ret=0
    case $state in
    (geectl)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:geectl-command-$line[1]:"
        case $line[1] in
            (version)
_arguments "${_arguments_options[@]}" : \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(config)
_arguments "${_arguments_options[@]}" : \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
":: :_geectl__subcmd__config_commands" \
"*::: :->config" \
&& ret=0

    case $state in
    (config)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:geectl-config-command-$line[1]:"
        case $line[1] in
            (show)
_arguments "${_arguments_options[@]}" : \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(init)
_arguments "${_arguments_options[@]}" : \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(import-oscrc)
_arguments "${_arguments_options[@]}" : \
'--oscrc=[The oscrc file to read (default \`\$XDG_CONFIG_HOME/osc/oscrc\`, else \`~/.config/osc/oscrc\`)]:OSCRC:_files' \
'--apiurl=[The apiurl section to import. Required when the file holds more than one section with a username; the only such section otherwise]:APIURL:_default' \
'--alias=[The instance alias to write. Defaults to the apiurl host with non-alphanumerics as \`_\`]:ALIAS:_default' \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--force[Replace the alias when it already names another URL]' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(help)
_arguments "${_arguments_options[@]}" : \
":: :_geectl__subcmd__config__subcmd__help_commands" \
"*::: :->help" \
&& ret=0

    case $state in
    (help)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:geectl-config-help-command-$line[1]:"
        case $line[1] in
            (show)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(init)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(import-oscrc)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(help)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
        esac
    ;;
esac
;;
        esac
    ;;
esac
;;
(remote)
_arguments "${_arguments_options[@]}" : \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
":: :_geectl__subcmd__remote_commands" \
"*::: :->remote" \
&& ret=0

    case $state in
    (remote)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:geectl-remote-command-$line[1]:"
        case $line[1] in
            (cat)
_arguments "${_arguments_options[@]}" : \
'--rev=[Print the file as of this revision (default\: latest)\: a revision number or a srcmd5]:REV:_default' \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
':project -- The project the package lives in:_default' \
':package -- The package the file lives in:_default' \
':file -- The file to print:_default' \
&& ret=0
;;
(download)
_arguments "${_arguments_options[@]}" : \
'--rev=[Download the file as of this revision\: a revision number or a srcmd5 (default\: latest)]:REV:_default' \
'--output=[Where to write the downloaded bytes]:OUTPUT:_files' \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
':project -- The project the package lives in:_default' \
':package -- The package the file lives in:_default' \
':file -- The file to download:_default' \
&& ret=0
;;
(list-files)
_arguments "${_arguments_options[@]}" : \
'--rev=[List the files as of this revision (default\: latest)\: a revision number or a srcmd5]:REV:_default' \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--include-deleted[Include files from deleted revisions in the listing\: the files of a deleted package, for asking whether a project ever held it. Requires OBS credentials\: the deleted listing reads the authenticated source route]' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
':project -- The project the package lives in:_default' \
':package -- The package to list:_default' \
&& ret=0
;;
(list-packages)
_arguments "${_arguments_options[@]}" : \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
':project -- The project to list:_default' \
&& ret=0
;;
(list-projects)
_arguments "${_arguments_options[@]}" : \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(list-staging-projects)
_arguments "${_arguments_options[@]}" : \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--no-requests[Leave out the staged, untracked, and obsolete request lists]' \
'--no-status[Leave out the overall state of each staging project]' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
':project -- The workflow project the staging projects belong to:_default' \
&& ret=0
;;
(staging-project)
_arguments "${_arguments_options[@]}" : \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--no-requests[Leave out the staged, untracked, and obsolete request lists]' \
'--no-status[Leave out the overall state of the staging project]' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
':project -- The workflow project the staging project belongs to:_default' \
':name -- The staging project name:_default' \
&& ret=0
;;
(package-log)
_arguments "${_arguments_options[@]}" : \
'--range=[Only entries in this revision range (\`FROM\:TO\`, either side may be empty; a side is a revision number or a srcmd5)]:RANGE:_default' \
'--limit=[At most this many entries]:LIMIT:_default' \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
':project -- The project the package lives in:_default' \
':package -- The package to show the history of:_default' \
&& ret=0
;;
(revision-patch)
_arguments "${_arguments_options[@]}" : \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
':project -- The project the package lives in:_default' \
':package -- The package to diff:_default' \
':rev -- The revision whose changes to show:_default' \
&& ret=0
;;
(project-meta)
_arguments "${_arguments_options[@]}" : \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
':project -- The project to show:_default' \
&& ret=0
;;
(set-project-meta)
_arguments "${_arguments_options[@]}" : \
'(--title --description --url --scmsync --devel-project --add-person --remove-person)--from-file=[Write the given meta file as the whole meta instead of setting fields\: the file is a \`<project>\` document whose name matches the project. Conflicts with every field and person flag]:FROM_FILE:_files' \
'--title=[The new title]:TITLE:_default' \
'--description=[The new description]:DESCRIPTION:_default' \
'--url=[The new URL]:URL:_default' \
'--scmsync=[The new scmsync URL]:SCMSYNC:_default' \
'--devel-project=[The new devel project]:DEVEL_PROJECT:_default' \
'*--add-person=[Grant a role to a person\: \`LOGIN\:ROLE\`, repeatable]:ADD_PERSON:_default' \
'*--remove-person=[Remove a person\: every role the login holds, repeatable]:REMOVE_PERSON:_default' \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help (see more with '\''--help'\'')]' \
'--help[Print help (see more with '\''--help'\'')]' \
':project -- The project whose meta is written:_default' \
&& ret=0
;;
(package-meta)
_arguments "${_arguments_options[@]}" : \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
':project -- The project the package lives in:_default' \
':package -- The package to show:_default' \
&& ret=0
;;
(set-package-meta)
_arguments "${_arguments_options[@]}" : \
'(--title --description --url --scmsync --releasename --devel-project --devel-package --add-person --remove-person)--from-file=[Write the given meta file as the whole meta instead of setting fields\: the file is a \`<package>\` document whose name and project match the package. Conflicts with every field and person flag]:FROM_FILE:_files' \
'--title=[The new title]:TITLE:_default' \
'--description=[The new description]:DESCRIPTION:_default' \
'--url=[The new URL]:URL:_default' \
'--scmsync=[The new scmsync URL]:SCMSYNC:_default' \
'--releasename=[The new release name]:RELEASENAME:_default' \
'--devel-project=[The new devel project]:DEVEL_PROJECT:_default' \
'--devel-package=[The new devel package. Needs \`--devel-project\` unless the package already has a devel project]:DEVEL_PACKAGE:_default' \
'*--add-person=[Grant a role to a person\: \`LOGIN\:ROLE\`, repeatable]:ADD_PERSON:_default' \
'*--remove-person=[Remove a person\: every role the login holds, repeatable]:REMOVE_PERSON:_default' \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help (see more with '\''--help'\'')]' \
'--help[Print help (see more with '\''--help'\'')]' \
':project -- The project the package lives in:_default' \
':package -- The package whose meta is written:_default' \
&& ret=0
;;
(update-package-meta-from-spec)
_arguments "${_arguments_options[@]}" : \
'--spec=[The spec file to read the meta from]:SPEC:_default' \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
':project -- The project the package lives in:_default' \
':package -- The package whose meta is updated:_default' \
&& ret=0
;;
(search-packages)
_arguments "${_arguments_options[@]}" : \
'--devel-project=[Only show packages developed in this project]:DEVEL_PROJECT:_default' \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--exact[Only show packages whose name is exactly the term]' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
'::term -- The search term; required unless `--devel-project` is given:_default' \
&& ret=0
;;
(search-projects)
_arguments "${_arguments_options[@]}" : \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--exact[Only show projects whose name is exactly the term]' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
':term -- The search term:_default' \
&& ret=0
;;
(search-projects-by-person)
_arguments "${_arguments_options[@]}" : \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
':userid -- The login to search projects for:_default' \
&& ret=0
;;
(search-packages-by-person)
_arguments "${_arguments_options[@]}" : \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
':userid -- The login to search packages for:_default' \
&& ret=0
;;
(count-scmsync-projects)
_arguments "${_arguments_options[@]}" : \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
':url_prefix -- The git URL prefix to count projects for:_default' \
&& ret=0
;;
(list-scmsync-projects)
_arguments "${_arguments_options[@]}" : \
'--limit=[At most this many projects (default 100)]:LIMIT:_default' \
'--offset=[Skip this many matching projects (default 0)]:OFFSET:_default' \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
':url_prefix -- The git URL prefix to list projects for:_default' \
&& ret=0
;;
(owners)
_arguments "${_arguments_options[@]}" : \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--verbose[Show each person as \`login (realname <email>)\`]' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
':project -- The project the package lives in:_default' \
':package -- The package to show the owners of:_default' \
&& ret=0
;;
(owners-for-binary)
_arguments "${_arguments_options[@]}" : \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--verbose[Show each person as \`login (realname <email>)\`]' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
':binary -- The binary package name:_default' \
&& ret=0
;;
(owners-for-package)
_arguments "${_arguments_options[@]}" : \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--verbose[Show each person as \`login (realname <email>)\`]' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
':package -- The package name:_default' \
&& ret=0
;;
(maintainers)
_arguments "${_arguments_options[@]}" : \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
':project -- The project the package lives in:_default' \
':package -- The package to show the maintainers of:_default' \
&& ret=0
;;
(whois)
_arguments "${_arguments_options[@]}" : \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
'::login -- The login name to look up; without one, the authenticated user:_default' \
&& ret=0
;;
(link-target)
_arguments "${_arguments_options[@]}" : \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
':project -- The project the package lives in:_default' \
':package -- The package to show the link target of:_default' \
&& ret=0
;;
(project-diff)
_arguments "${_arguments_options[@]}" : \
'*--include=[Only include packages with these exact names (repeatable)]:INCLUDE:_default' \
'*--exclude=[Exclude packages with these exact names (repeatable)]:EXCLUDE:_default' \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--annotate-requests[Annotate differing packages with their open submit requests]' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
':old -- The old project:_default' \
':new -- The new project:_default' \
&& ret=0
;;
(diff-packages)
_arguments "${_arguments_options[@]}" : \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
':old_project -- The old project:_default' \
':old_package -- The old package:_default' \
':new_project -- The new project:_default' \
':new_package -- The new package:_default' \
&& ret=0
;;
(devel-project)
_arguments "${_arguments_options[@]}" : \
'--target-project=[The target project the package lives in (default\: openSUSE\:Factory)]:TARGET_PROJECT:_default' \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
':package -- The package to look up:_default' \
&& ret=0
;;
(repo-add)
_arguments "${_arguments_options[@]}" : \
'*--arch=[An architecture the repository builds for (repeatable; at least one is required)]:ARCH:_default' \
'*--path=[A rebuild source as \`PROJECT/REPOSITORY\` (repeatable; at least one is required)]:PATH:_default' \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--disable-publish[Disable publishing for the added repository]' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
':project -- The project to add the repository to:_default' \
':name -- The repository name:_default' \
&& ret=0
;;
(repo-remove)
_arguments "${_arguments_options[@]}" : \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
':project -- The project to remove the repository from:_default' \
':name -- The repository name:_default' \
&& ret=0
;;
(upload-file)
_arguments "${_arguments_options[@]}" : \
'--from=[The local file to read the content from]:FROM:_files' \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--binary[Upload bytes instead of text\: a binary file uploads instead of failing the UTF-8 check a text upload would hit]' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
':project -- The project the package lives in:_default' \
':package -- The package to upload into:_default' \
':file -- The file name on the server:_default' \
&& ret=0
;;
(create-project)
_arguments "${_arguments_options[@]}" : \
'--title=[The project title]:TITLE:_default' \
'--description=[The project description]:DESCRIPTION:_default' \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
':project -- The project to create:_default' \
&& ret=0
;;
(create-package)
_arguments "${_arguments_options[@]}" : \
'--title=[The package title]:TITLE:_default' \
'--description=[The package description]:DESCRIPTION:_default' \
'--scmsync=[The git repository and branch the package syncs its sources from, as \`url#branch\`]:SCMSYNC:_default' \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--force[Overwrite the package when it already exists]' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
':project -- The project to create the package in:_default' \
':package -- The package to create:_default' \
&& ret=0
;;
(create-token)
_arguments "${_arguments_options[@]}" : \
'--operation=[The operation the token authorises\: runservice, rebuild, release, or workflow]:OPERATION:((runservice\:"Run source services\: the \`runservice\` trigger"
rebuild\:"Rebuild packages\: the \`rebuild\` trigger"
release\:"Release packages\: the \`release\` trigger"
workflow\:"Run SCM/CI workflows\: the \`workflow\` trigger"))' \
'--project=[The project of the package the token is scoped to]:PROJECT:_default' \
'--package=[The package the token is scoped to]:PACKAGE:_default' \
'--description=[A description identifying the token, at most 64 characters]:DESCRIPTION:_default' \
'--scm-token=[The SCM personal access token OBS uses to talk to the SCM on the user'\''s behalf. Workflow tokens only. The flag wins over the environment; without it, \`GEECTL_SCM_TOKEN\` (or \`GEECTL_SCM_TOKEN_FILE\`, a file whose whole content is the token) supplies it, so the secret stays out of \`ps\` and shell history]:SCM_TOKEN:_default' \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help (see more with '\''--help'\'')]' \
'--help[Print help (see more with '\''--help'\'')]' \
&& ret=0
;;
(delete-package)
_arguments "${_arguments_options[@]}" : \
'--message=[A message recorded on the delete, sent as the server'\''s \`comment\` query parameter]:MESSAGE:_default' \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--force[Delete even when open submit requests involve the package]' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
':project -- The project the package lives in:_default' \
':package -- The package to delete:_default' \
&& ret=0
;;
(delete-link)
_arguments "${_arguments_options[@]}" : \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
':project -- The project the package lives in:_default' \
':package -- The package whose `_link` is deleted:_default' \
&& ret=0
;;
(delete-file)
_arguments "${_arguments_options[@]}" : \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
':project -- The project the package lives in:_default' \
':package -- The package the file lives in:_default' \
':file -- The file to delete:_default' \
&& ret=0
;;
(delete-project)
_arguments "${_arguments_options[@]}" : \
'--comment=[A comment recorded on the delete]:COMMENT:_default' \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--recursive[Delete the project with all its packages]' \
'--force[Ignore other projects depending on this one]' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
':project -- The project to delete:_default' \
&& ret=0
;;
(undelete-package)
_arguments "${_arguments_options[@]}" : \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
':project -- The project the package lives in:_default' \
':package -- The package to restore:_default' \
&& ret=0
;;
(undelete-project)
_arguments "${_arguments_options[@]}" : \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
':project -- The project to restore:_default' \
&& ret=0
;;
(aggregate-package)
_arguments "${_arguments_options[@]}" : \
'*--repository-map=[Map a source repository to a target repository, as \`SOURCE=TARGET\` (repeatable)]:REPOSITORY_MAP:_default' \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--no-sources[Aggregate binaries only, not the sources]' \
'--disable-publish[Disable publishing of the aggregate package]' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
':source_project -- The project the source package lives in:_default' \
':source_package -- The source package:_default' \
':target_project -- The project the target package lives in:_default' \
':target_package -- The target package:_default' \
&& ret=0
;;
(release-package)
_arguments "${_arguments_options[@]}" : \
'--repository=[Release only binaries from this repository]:REPOSITORY:_default' \
'--arch=[Release only binaries for this architecture]:ARCH:_default' \
'--target-project=[Release only to this project]:TARGET_PROJECT:_default' \
'--target-repository=[Release only to this repository in the target project]:TARGET_REPOSITORY:_default' \
'--set-release=[Rename binaries during the release using this release tag]:SET_RELEASE:_default' \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--no-delay[Run the release immediately instead of queueing it as a job]' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
':project -- The project the package lives in:_default' \
':package -- The package to release:_default' \
&& ret=0
;;
(release-project)
_arguments "${_arguments_options[@]}" : \
'--repository=[Release only binaries from this repository]:REPOSITORY:_default' \
'--arch=[Release only binaries for this architecture]:ARCH:_default' \
'--target-project=[Release only to this project]:TARGET_PROJECT:_default' \
'--target-repository=[Release only to this repository in the target project]:TARGET_REPOSITORY:_default' \
'--set-release=[Rename binaries during the release using this release tag]:SET_RELEASE:_default' \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--no-delay[Run the release immediately instead of queueing it as a job]' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
':project -- The project to release:_default' \
&& ret=0
;;
(lock)
_arguments "${_arguments_options[@]}" : \
'--message=[A message for the lock, sent as the server'\''s \`comment\` query parameter. A project lock records it; a package lock sends it but the server does not record it — there is no package-scope lock command]:MESSAGE:_default' \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
':project -- The project to lock:_default' \
'::package -- The package to lock. Omit to lock the whole project:_default' \
&& ret=0
;;
(unlock)
_arguments "${_arguments_options[@]}" : \
'--message=[A message recorded on the unlock, sent as the server'\''s \`comment\` query parameter]:MESSAGE:_default' \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
':project -- The project to unlock:_default' \
'::package -- The package to unlock. Omit to unlock the whole project:_default' \
&& ret=0
;;
(add-channels)
_arguments "${_arguments_options[@]}" : \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'(--skip-disabled)--enable-all[Enable all added channels including the ones disabled by default. Cannot combine with \`--skip-disabled\`]' \
'(--enable-all)--skip-disabled[Skip disabled channels\: add the channel packages but not their repositories. Cannot combine with \`--enable-all\`]' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
':project -- The project to add channels to:_default' \
'::package -- The package to add channels to. Omit for the whole project:_default' \
&& ret=0
;;
(add-containers)
_arguments "${_arguments_options[@]}" : \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--extend-package-names[Extend the package names with the container names]' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
':project -- The project the package lives in:_default' \
':package -- The package to add containers for:_default' \
&& ret=0
;;
(help)
_arguments "${_arguments_options[@]}" : \
":: :_geectl__subcmd__remote__subcmd__help_commands" \
"*::: :->help" \
&& ret=0

    case $state in
    (help)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:geectl-remote-help-command-$line[1]:"
        case $line[1] in
            (cat)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(download)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(list-files)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(list-packages)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(list-projects)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(list-staging-projects)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(staging-project)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(package-log)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(revision-patch)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(project-meta)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(set-project-meta)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(package-meta)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(set-package-meta)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(update-package-meta-from-spec)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(search-packages)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(search-projects)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(search-projects-by-person)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(search-packages-by-person)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(count-scmsync-projects)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(list-scmsync-projects)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(owners)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(owners-for-binary)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(owners-for-package)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(maintainers)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(whois)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(link-target)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(project-diff)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(diff-packages)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(devel-project)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(repo-add)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(repo-remove)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(upload-file)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(create-project)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(create-package)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(create-token)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(delete-package)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(delete-link)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(delete-file)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(delete-project)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(undelete-package)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(undelete-project)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(aggregate-package)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(release-package)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(release-project)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(lock)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(unlock)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(add-channels)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(add-containers)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(help)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
        esac
    ;;
esac
;;
        esac
    ;;
esac
;;
(request)
_arguments "${_arguments_options[@]}" : \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
":: :_geectl__subcmd__request_commands" \
"*::: :->request" \
&& ret=0

    case $state in
    (request)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:geectl-request-command-$line[1]:"
        case $line[1] in
            (show)
_arguments "${_arguments_options[@]}" : \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
':id -- The request id:_default' \
&& ret=0
;;
(log)
_arguments "${_arguments_options[@]}" : \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
':id -- The request id:_default' \
&& ret=0
;;
(search)
_arguments "${_arguments_options[@]}" : \
'*--request-id=[Only requests with these ids]:REQUEST_IDS:_default' \
'*--project=[Only requests with an action from or to these projects]:PROJECT:_default' \
'*--package=[Only requests with an action from or to these packages]:PACKAGE:_default' \
'*--source-project=[Only requests with an action from these source projects]:SOURCE_PROJECT:_default' \
'*--source-package=[Only requests with an action from these source packages]:SOURCE_PACKAGE:_default' \
'*--target-project=[Only requests with an action to these target projects]:TARGET_PROJECT:_default' \
'*--target-package=[Only requests with an action to these target packages]:TARGET_PACKAGE:_default' \
'*--creator=[Only requests filed by these users]:CREATOR:_default' \
'*--who=[Only requests involving these users]:WHO:_default' \
'*--reviewer=[Only requests with an open review addressed to these users. With no \`--state\`, only new and review requests are listed]:REVIEWER:_default' \
'*--state=[Only requests in these states. When none are given, only new, review and declined requests are listed; \`--reviewer\` narrows this to new and review]:STATE:(new review accepted declined revoked superseded deleted)' \
'*--type=[Only requests with an action of these types]:TYPES:_default' \
'*--exclude-target-project=[Exclude requests with an action targeting these projects]:EXCLUDE_TARGET_PROJECT:_default' \
'--since=[Only requests with a history entry at or after this date, or a last update at or after it, as \`YYYY-MM-DD\` or \`YYYY-MM-DDTHH\:MM\:SS\`]:SINCE:_default' \
'--limit=[Ask the server for at most this many matches]:LIMIT:_default' \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--with-history[Include the history of each request]' \
'--with-full-history[Include the full history of each request]' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(list)
_arguments "${_arguments_options[@]}" : \
'*--project=[Requests touching this project]:PROJECT:_default' \
'*--package=[Requests touching this package; requires \`--project\`]:PACKAGE:_default' \
'*--state=[Requests in this state; when empty, new and review]:STATE:_default' \
'--since=[Only requests with a history entry at or after this date, or a last update at or after it, as \`YYYY-MM-DD\` or \`YYYY-MM-DDTHH\:MM\:SS\`]:SINCE:_default' \
'--limit=[Ask the server for at most this many matches]:LIMIT:_default' \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(diff)
_arguments "${_arguments_options[@]}" : \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--xml[Render the request XML with the diff injected, instead of the unified diff]' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
':id -- The request id:_default' \
&& ret=0
;;
(list-comments)
_arguments "${_arguments_options[@]}" : \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
':id -- The request id:_default' \
&& ret=0
;;
(accept)
_arguments "${_arguments_options[@]}" : \
'--comment=[The message left with the accept. Values starting with \`-\` are taken literally\: a comment may open with a \`- \` bullet]:COMMENT:_default' \
'-m+[The changes entry for the forwarded request, overriding the original description. Values starting with \`-\` are taken literally\: an entry may open with a \`- \` bullet]:FORWARD_MESSAGE:_default' \
'--forward-message=[The changes entry for the forwarded request, overriding the original description. Values starting with \`-\` are taken literally\: an entry may open with a \`- \` bullet]:FORWARD_MESSAGE:_default' \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--force[Accept a request still in \`review\` state, overriding its open reviews]' \
'--no-forward[Do not forward the request after accepting it\: by default an accepted request is forwarded to its link parent]' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
':id -- The request id:_default' \
&& ret=0
;;
(decline)
_arguments "${_arguments_options[@]}" : \
'--comment=[The reason for the decline. Values starting with \`-\` are taken literally\: a comment may open with a \`- \` bullet]:COMMENT:_default' \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
':id -- The request id:_default' \
&& ret=0
;;
(revoke)
_arguments "${_arguments_options[@]}" : \
'--comment=[The reason for the revocation. Values starting with \`-\` are taken literally\: a comment may open with a \`- \` bullet]:COMMENT:_default' \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
':id -- The request id:_default' \
&& ret=0
;;
(change-state)
_arguments "${_arguments_options[@]}" : \
'--comment=[The message left with the change. Values starting with \`-\` are taken literally\: a comment may open with a \`- \` bullet]:COMMENT:_default' \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--force[Force the change, overriding open reviews. Only honoured when changing to accepted]' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
':id -- The request id:_default' \
':state -- The state to change to:(new review accepted declined revoked)' \
&& ret=0
;;
(try-change-state)
_arguments "${_arguments_options[@]}" : \
'--comment=[The message left with the change. Values starting with \`-\` are taken literally\: a comment may open with a \`- \` bullet]:COMMENT:_default' \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--force[Force the change, overriding open reviews. Only honoured when changing to accepted]' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
':id -- The request id:_default' \
':state -- The state to change to:(new review accepted declined revoked)' \
&& ret=0
;;
(add-review)
_arguments "${_arguments_options[@]}" : \
'--user=[The user the review is asked of]:USER:_default' \
'--group=[The group the review is asked of]:GROUP:_default' \
'--project=[The project the review is asked of]:PROJECT:_default' \
'--package=[The package the review is asked of (needs its project)]:PACKAGE:_default' \
'--comment=[The reason the review is asked for. Values starting with \`-\` are taken literally\: a comment may open with a \`- \` bullet]:COMMENT:_default' \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
':id -- The request id:_default' \
&& ret=0
;;
(change-review-state)
_arguments "${_arguments_options[@]}" : \
'--user=[The user the review was asked of]:USER:_default' \
'--group=[The group the review was asked of]:GROUP:_default' \
'--project=[The project the review was asked of]:PROJECT:_default' \
'--package=[The package the review was asked of (needs its project)]:PACKAGE:_default' \
'--comment=[The message left with the change. Values starting with \`-\` are taken literally\: a comment may open with a \`- \` bullet]:COMMENT:_default' \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
':id -- The request id:_default' \
':state -- The state to change the review to:(new accepted declined)' \
&& ret=0
;;
(add-comment)
_arguments "${_arguments_options[@]}" : \
'--body=[The comment text. Values starting with \`-\` are taken literally\: the body may open with a \`- \` bullet]:BODY:_default' \
'--parent=[The comment being replied to, when this is a reply]:PARENT:_default' \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
':id -- The request id:_default' \
&& ret=0
;;
(file)
_arguments "${_arguments_options[@]}" : \
'--source-project=[The project the change comes from (the branch)]:SOURCE_PROJECT:_default' \
'--source-package=[The package in the source project]:SOURCE_PACKAGE:_default' \
'--target-project=[The project the request targets\: the devel project, or openSUSE\:Factory when the source is the recorded devel project and package]:TARGET_PROJECT:_default' \
'--target-package=[The package in the target project]:TARGET_PACKAGE:_default' \
'--description=[The request description\: one headline, then \`- \` lines for staging, needs, and rebuilds. Values starting with \`-\` are taken literally\: a description may open with a \`- \` bullet]:DESCRIPTION:_default' \
'--changes-entry=[The one \`.changes\` entry for this submission. It must name every CVE passed with \`--cve\`. Values starting with \`-\` are taken literally\: a \`.changes\` entry starts with \`- \`]:CHANGES_ENTRY:_default' \
'*--cve=[A CVE this submission claims to fix. Repeatable; each one must be named in \`--changes-entry\`]:CVE:_default' \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--without-cleanup[Keep the source branch on accept instead of deleting it (cleanup is the default)]' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(refile)
_arguments "${_arguments_options[@]}" : \
'--source-project=[The project the change comes from (the branch)]:SOURCE_PROJECT:_default' \
'--source-package=[The package in the source project]:SOURCE_PACKAGE:_default' \
'--target-project=[The project the request targets\: the devel project, or openSUSE\:Factory when the source is the recorded devel project and package]:TARGET_PROJECT:_default' \
'--target-package=[The package in the target project]:TARGET_PACKAGE:_default' \
'--description=[The request description\: one headline, then \`- \` lines for staging, needs, and rebuilds. Values starting with \`-\` are taken literally\: a description may open with a \`- \` bullet]:DESCRIPTION:_default' \
'--changes-entry=[The one \`.changes\` entry for this submission. It must name every CVE passed with \`--cve\`. Values starting with \`-\` are taken literally\: a \`.changes\` entry starts with \`- \`]:CHANGES_ENTRY:_default' \
'*--cve=[A CVE this submission claims to fix. Repeatable; each one must be named in \`--changes-entry\`]:CVE:_default' \
'--supersede=[The open request this filing supersedes]:SUPERSEDE:_default' \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--without-cleanup[Keep the source branch on accept instead of deleting it (cleanup is the default)]' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(supersede)
_arguments "${_arguments_options[@]}" : \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
':old_id -- The open request being superseded:_default' \
':new_id -- The existing request superseding it:_default' \
&& ret=0
;;
(file-add-role)
_arguments "${_arguments_options[@]}" : \
'--target-project=[The project the role is granted on]:TARGET_PROJECT:_default' \
'--target-package=[The package the role is granted on; omit for a project-level grant]:TARGET_PACKAGE:_default' \
'--person=[The person granted the role]:PERSON:_default' \
'--person-role=[The role granted to the person]:PERSON_ROLE:_default' \
'--group=[The group granted the role]:GROUP:_default' \
'--group-role=[The role granted to the group]:GROUP_ROLE:_default' \
'--description=[The request description. Values starting with \`-\` are taken literally\: a description may open with a \`- \` bullet]:DESCRIPTION:_default' \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(file-change-devel)
_arguments "${_arguments_options[@]}" : \
'--source-project=[The project the package lives in]:SOURCE_PROJECT:_default' \
'--source-package=[The package whose devel project changes]:SOURCE_PACKAGE:_default' \
'--target-project=[The project that becomes the devel project]:TARGET_PROJECT:_default' \
'--target-package=[The package in the devel project]:TARGET_PACKAGE:_default' \
'--description=[The request description. Values starting with \`-\` are taken literally\: a description may open with a \`- \` bullet]:DESCRIPTION:_default' \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(file-delete)
_arguments "${_arguments_options[@]}" : \
'--target-project=[The project the deletion targets]:TARGET_PROJECT:_default' \
'(--repository)--package=[The package to delete. Omit both this and \`--repository\` to delete the whole project (with \`--whole-project\`)]:PACKAGE:_default' \
'--repository=[The repository to delete. Omit both this and \`--package\` to delete the whole project (with \`--whole-project\`)]:REPOSITORY:_default' \
'--description=[The request description. Values starting with \`-\` are taken literally\: a description may open with a \`- \` bullet]:DESCRIPTION:_default' \
'--accept-in-hours=[Auto-accept the request this many hours after filing (the server accepts it on its own once the time passes), as \`osc deletereq --accept-in-hours\` does. The timestamp is computed at filing time; 0 means accept immediately]:ACCEPT_IN_HOURS:_default' \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'(--package --repository)--whole-project[Delete the whole project\: required when neither \`--package\` nor \`--repository\` is given. A whole-project deletion is destructive, so omitting the target is not enough — the flag is the explicit confirmation]' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(file-maintenance-incident)
_arguments "${_arguments_options[@]}" : \
'--source-project=[The project the maintenance update comes from]:SOURCE_PROJECT:_default' \
'*--source-package=[A package in the source project. Repeatable\: one action per package; at least one is required]:SOURCE_PACKAGE:_default' \
'--target-project=[The maintenance project the incident goes to]:TARGET_PROJECT:_default' \
'--release-project=[The release project the update releases into. When omitted, the server resolves it from the maintenance project]:RELEASE_PROJECT:_default' \
'--description=[The request description. Values starting with \`-\` are taken literally\: a description may open with a \`- \` bullet]:DESCRIPTION:_default' \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--with-cleanup[Delete the source branch when the incident is accepted. By default the branch survives the incident]' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(refile-maintenance-incident)
_arguments "${_arguments_options[@]}" : \
'--source-project=[The project the maintenance update comes from]:SOURCE_PROJECT:_default' \
'*--source-package=[A package in the source project. Repeatable\: one action per package; at least one is required]:SOURCE_PACKAGE:_default' \
'--target-project=[The maintenance project the incident goes to]:TARGET_PROJECT:_default' \
'--release-project=[The release project the update releases into. When omitted, the server resolves it from the maintenance project]:RELEASE_PROJECT:_default' \
'--description=[The request description. Values starting with \`-\` are taken literally\: a description may open with a \`- \` bullet]:DESCRIPTION:_default' \
'--supersede=[The open request this filing supersedes]:SUPERSEDE:_default' \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--with-cleanup[Delete the source branch when the incident is accepted. By default the branch survives the incident]' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(help)
_arguments "${_arguments_options[@]}" : \
":: :_geectl__subcmd__request__subcmd__help_commands" \
"*::: :->help" \
&& ret=0

    case $state in
    (help)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:geectl-request-help-command-$line[1]:"
        case $line[1] in
            (show)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(log)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(search)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(list)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(diff)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(list-comments)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(accept)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(decline)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(revoke)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(change-state)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(try-change-state)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(add-review)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(change-review-state)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(add-comment)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(file)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(refile)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(supersede)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(file-add-role)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(file-change-devel)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(file-delete)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(file-maintenance-incident)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(refile-maintenance-incident)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(help)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
        esac
    ;;
esac
;;
        esac
    ;;
esac
;;
(branch)
_arguments "${_arguments_options[@]}" : \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
":: :_geectl__subcmd__branch_commands" \
"*::: :->branch" \
&& ret=0

    case $state in
    (branch)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:geectl-branch-command-$line[1]:"
        case $line[1] in
            (create)
_arguments "${_arguments_options[@]}" : \
'--project=[The project to branch from]:PROJECT:_default' \
'--package=[The package to branch]:PACKAGE:_default' \
'--new-repo-name=[Name for the fork; defaults to the source repo name]:NEW_REPO_NAME:_default' \
'--organization=[Organization to fork into; defaults to the token owner'\''s account]:ORGANIZATION:_default' \
'--git-branch=[Branch to fork from and name in the fork'\''s scmsync URL; defaults to the source branch]:GIT_BRANCH:_default' \
'--target-project=[The branch project. Without it, the server picks the default (usually \`home\:<user>\:branches\:<source project>\`)]:TARGET_PROJECT:_default' \
'--target-package=[The branch package name. Without it, the source package name]:TARGET_PACKAGE:_default' \
'--first-build-deadline-secs=[Seconds to wait for the first build of the forked package in the fork'\''s test project; 0 skips the wait and reports the branch without a build. No wait by default]:FIRST_BUILD_DEADLINE_SECS:_default' \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--no-devel-project[Ignore the devel project when resolving the fork source]' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(branch-package)
_arguments "${_arguments_options[@]}" : \
'--project=[The project to branch from]:PROJECT:_default' \
'--package=[The package to branch]:PACKAGE:_default' \
'--target-project=[The branch project. Without it, the server picks the default (usually \`home\:<user>\:branches\:<source project>\`)]:TARGET_PROJECT:_default' \
'--target-package=[The branch package name. Without it, the source package name]:TARGET_PACKAGE:_default' \
'--revision=[Branch from this source revision instead of the latest]:REVISION:_default' \
'--linkrev=[The link revision the branch records]:LINKREV:_default' \
'--comment=[A comment recorded on the branch]:COMMENT:_default' \
'--add-repositories-block=[Block the added repositories. Only sent with \`--add-repositories\` (clap refuses it without that flag\: a block the live run would silently drop must fail at parse time, not vanish)]:ADD_REPOSITORIES_BLOCK:_default' \
'--add-repositories-rebuild=[Rebuild the added repositories. Only sent with \`--add-repositories\` (clap refuses it without that flag, as above)]:ADD_REPOSITORIES_REBUILD:_default' \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--no-devel-project[Ignore the devel project when resolving the link source]' \
'--new-package[Branch a package that does not exist yet]' \
'--force[Overwrite an existing target package]' \
'--noaccess[Withhold the branch'\''s read access to the source package'\''s project]' \
'--add-repositories[Add the source project'\''s repositories to the branch project]' \
'--extend-package-names[Extend the branch package names with the source project name]' \
'--maintenance[Branch as a maintenance incident]' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(copy-package)
_arguments "${_arguments_options[@]}" : \
'--source-project=[The project to copy from]:SOURCE_PROJECT:_default' \
'--source-package=[The package to copy]:SOURCE_PACKAGE:_default' \
'--target-project=[The project to copy to]:TARGET_PROJECT:_default' \
'--target-package=[The package to copy to]:TARGET_PACKAGE:_default' \
'--revision=[Copy this source revision instead of the latest]:REVISION:_default' \
'--comment=[A comment recorded on the copy]:COMMENT:_default' \
'--to-apiurl=[Copy to another OBS instance at this API URL instead of the configured one. The target'\''s credentials come from the instance alias matching that URL in the config file'\''s \`\[instances\]\` table (\`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\` or \`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\`; see \`auth\:\:resolve_obs_for_url\`) and the copy fails clearly when none are configured — the source instance'\''s credentials are never forwarded to the target. The copy is client-side and always expanded; \`--keep-link\` and the server-side copy options are refused with it]:TO_APIURL:_default' \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--keep-link[Keep the source package'\''s link when copying a link package]' \
'--expand[Copy the expanded source (the link resolved) instead of the link itself]' \
'--noservice[Do not run source services on the copied files]' \
'--dont-update-source[Do not update the source package when copying a link onto its target]' \
'--with-vrev[Carry the source revision'\''s vrev over to the copy]' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(copy-project)
_arguments "${_arguments_options[@]}" : \
'--source-project=[The project to copy from]:SOURCE_PROJECT:_default' \
'--target-project=[The project to copy to]:TARGET_PROJECT:_default' \
'--comment=[A comment recorded on the copy]:COMMENT:_default' \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--with-binaries[Copy the built binaries as well. Admin-only on the server]' \
'--with-history[Copy the project history as well]' \
'--make-older[Make the copied project older than the source]' \
'--make-origin-older[Make the copied project older than the source'\''s origin]' \
'--noservice[Do not run source services on the copied files]' \
'--resign[Re-sign the copied project]' \
'--no-delay[Run the copy synchronously instead of as a background job]' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(set-devel-project)
_arguments "${_arguments_options[@]}" : \
'--project=[The project holding the package]:PROJECT:_default' \
'--package=[The package whose devel project to set]:PACKAGE:_default' \
'--devel-project=[The devel project. Without it, the devel project is removed]:DEVEL_PROJECT:_default' \
'--devel-package=[The devel package. Without it, the devel project alone is set]:DEVEL_PACKAGE:_default' \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(link-package)
_arguments "${_arguments_options[@]}" : \
'--source-project=[The project to link from]:SOURCE_PROJECT:_default' \
'--source-package=[The package to link]:SOURCE_PACKAGE:_default' \
'--target-project=[The project to link in]:TARGET_PROJECT:_default' \
'--target-package=[The package to link as]:TARGET_PACKAGE:_default' \
'--revision=[Pin the link at this source revision (a number or an md5)]:REVISION:_default' \
'--vrev=[The link'\''s vrev attribute]:VREV:_default' \
'--cicount=[The link'\''s cicount attribute\: \`add\`, \`copy\`, or \`local\`]:CICOUNT:(add copy local)' \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--force[Replace an existing link in the target package]' \
'--missing-target[Allow the link to point at a package that does not exist yet]' \
'--disable-build[Disable building the target package]' \
'--disable-publish[Disable publishing the target package]' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(repair-link)
_arguments "${_arguments_options[@]}" : \
'--project=[The project holding the package]:PROJECT:_default' \
'--package=[The package whose link to repair]:PACKAGE:_default' \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(set-link-rev)
_arguments "${_arguments_options[@]}" : \
'--project=[The project holding the package]:PROJECT:_default' \
'--package=[The package whose link revision to set]:PACKAGE:_default' \
'--revision=[The revision\: \`latest\`, \`expanded\`, \`unset\`, a bare revision (a number or an md5, expanded), or \`pinned\:<rev>\` / \`expanded\:<rev>\`]:REVISION:_default' \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(set-link-rev-project)
_arguments "${_arguments_options[@]}" : \
'--project=[The project whose linked packages to set]:PROJECT:_default' \
'--revision=[The revision\: \`latest\`, \`expanded\`, or \`unset\`. A pinned revision needs a package\: use \`set-link-rev\` for one package at a time]:REVISION:_default' \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(link-to-branch)
_arguments "${_arguments_options[@]}" : \
'--project=[The project holding the package]:PROJECT:_default' \
'--package=[The package whose link to turn into a branch]:PACKAGE:_default' \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(detach-branch)
_arguments "${_arguments_options[@]}" : \
'--project=[The project holding the package]:PROJECT:_default' \
'--package=[The branch package to detach]:PACKAGE:_default' \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(help)
_arguments "${_arguments_options[@]}" : \
":: :_geectl__subcmd__branch__subcmd__help_commands" \
"*::: :->help" \
&& ret=0

    case $state in
    (help)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:geectl-branch-help-command-$line[1]:"
        case $line[1] in
            (create)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(branch-package)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(copy-package)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(copy-project)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(set-devel-project)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(link-package)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(repair-link)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(set-link-rev)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(set-link-rev-project)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(link-to-branch)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(detach-branch)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(help)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
        esac
    ;;
esac
;;
        esac
    ;;
esac
;;
(build)
_arguments "${_arguments_options[@]}" : \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
":: :_geectl__subcmd__build_commands" \
"*::: :->build" \
&& ret=0

    case $state in
    (build)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:geectl-build-command-$line[1]:"
        case $line[1] in
            (results)
_arguments "${_arguments_options[@]}" : \
'--package=[Only this package'\''s statuses (the repository/architecture cells still come back whole)]:PACKAGE:_default' \
'*--repository=[Only these repositories (repeatable)]:REPOSITORY:_default' \
'*--arch=[Only these architectures (repeatable)]:ARCH:_default' \
'--code=[Only packages with this build code (e.g. \`failed\`)]:CODE:_default' \
'--oldstate=[Poll for changes since this state hash (from a previous run). Only \`results\` takes it\: \`wait-for-results\` polls plainly]:OLDSTATE:_default' \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--multibuild[Expand multibuild flavors into their own statuses]' \
'--locallink[Follow local links when expanding multibuild flavors]' \
'--lastbuild[The last build'\''s results instead of the current state]' \
'(--oldstate)--all-instances[Merge the results from every configured instance into one view, instead of only the instance currently pointed at. Cannot combine with \`--oldstate\`\: a state hash is per instance. A partially-failing read still exits 0\: per-instance errors render in the view and payload, and only an every-instance failure is an error]' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
':project -- The project to read:_default' \
&& ret=0
;;
(wait-for-results)
_arguments "${_arguments_options[@]}" : \
'--package=[Only this package'\''s statuses (the repository/architecture cells still come back whole)]:PACKAGE:_default' \
'*--repository=[Only these repositories (repeatable)]:REPOSITORY:_default' \
'*--arch=[Only these architectures (repeatable)]:ARCH:_default' \
'--code=[Only packages with this build code (e.g. \`failed\`)]:CODE:_default' \
'--deadline-secs=[Deadline in seconds\: the wait fails with a timeout past it]:DEADLINE_SECS:_default' \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--multibuild[Expand multibuild flavors into their own statuses]' \
'--locallink[Follow local links when expanding multibuild flavors]' \
'--lastbuild[The last build'\''s results instead of the current state]' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
':project -- The project to poll:_default' \
&& ret=0
;;
(build-log)
_arguments "${_arguments_options[@]}" : \
'--start=[Start reading at this byte offset; a negative offset reads from the end of the log]:START:_default' \
'--multibuild=[Only this multibuild flavor'\''s log\: appends \`\:FLAVOR\` to the package. Refused when the package already carries a flavor]:FLAVOR:_default' \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'(--lastsucceeded)--last[The last build'\''s log instead of the current one. Cannot combine with \`--lastsucceeded\`]' \
'--lastsucceeded[The last succeeded build'\''s log. Cannot combine with \`--last\`]' \
'--strip-time[Strip the leading \`\[timestamp\] \` from each log line]' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
':project -- The project the package is in — or a build-log URL, in which case the other coordinates are omitted. The URL form takes an API log URL (`http(s)\://<api-host>/build/<project>/<repository>/<arch>/<package>/_log`) or a web-UI log URL (`http(s)\://build.opensuse.org/package/live_build_log/<project>/<package>/<repository>/<arch>`):_default' \
'::package -- The package to read the log for. Omit with a URL:_default' \
'::repository -- The repository the package builds in. Omit with a URL:_default' \
'::arch -- The architecture the package builds for. Omit with a URL:_default' \
&& ret=0
;;
(build-status)
_arguments "${_arguments_options[@]}" : \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
':project -- The project the package is in:_default' \
':package -- The package to read the status for:_default' \
':repository -- The repository the package builds in:_default' \
':arch -- The architecture the package builds for:_default' \
&& ret=0
;;
(job-status)
_arguments "${_arguments_options[@]}" : \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
':project -- The project the package is in:_default' \
':package -- The package to read the job for:_default' \
':repository -- The repository the package builds in:_default' \
':arch -- The architecture the package builds for:_default' \
&& ret=0
;;
(rebuild-package)
_arguments "${_arguments_options[@]}" : \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
':project -- The project the package is in:_default' \
':package -- The package to rebuild:_default' \
':repository -- The repository the package builds in:_default' \
':arch -- The architecture the package builds for:_default' \
&& ret=0
;;
(restart-build)
_arguments "${_arguments_options[@]}" : \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
':project -- The project the package is in:_default' \
':package -- The package to restart the build for:_default' \
':repository -- The repository the package builds in:_default' \
':arch -- The architecture the package builds for:_default' \
&& ret=0
;;
(abort-build)
_arguments "${_arguments_options[@]}" : \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
':project -- The project the package is in:_default' \
':package -- The package to abort the build for:_default' \
':repository -- The repository the package builds in:_default' \
':arch -- The architecture the package builds for:_default' \
&& ret=0
;;
(wipe-binaries)
_arguments "${_arguments_options[@]}" : \
'--arch=[Wipe only binaries for this architecture]:ARCH:_default' \
'--repository=[Wipe only binaries for this repository]:REPOSITORY:_default' \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--all[Wipe all binaries regardless of package status. Combines with no status filter]' \
'--build-disabled[Wipe only binaries of packages whose build is disabled]' \
'--build-failed[Wipe only binaries of packages whose build failed]' \
'--broken[Wipe only binaries of packages whose sources are bad]' \
'--unresolvable[Wipe only binaries of packages with dependency errors]' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
':project -- The project to wipe binaries in:_default' \
'::package -- The package to wipe binaries for. Omit to wipe the whole project:_default' \
&& ret=0
;;
(buildinfo)
_arguments "${_arguments_options[@]}" : \
'--alternative-project=[Read the buildinfo for another project instead of \`project\`\: the dep set the package would build against there]:ALTERNATIVE_PROJECT:_default' \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
':project -- The project the package is in:_default' \
':package -- The package to read the buildinfo for:_default' \
':repository -- The repository the package builds in:_default' \
':arch -- The architecture the package builds for:_default' \
&& ret=0
;;
(buildconfig)
_arguments "${_arguments_options[@]}" : \
'--alternative-project=[Read the build configuration for another project instead of \`project\`\: the expanded \`_buildconfig\` text the engine takes as \`--dist\` there]:ALTERNATIVE_PROJECT:_default' \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
':project -- The project to read the build configuration for:_default' \
':repository -- The repository to read the build configuration for:_default' \
&& ret=0
;;
(build-history)
_arguments "${_arguments_options[@]}" : \
'--limit=[At most this many entries]:LIMIT:_default' \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
':project -- The project the package is in:_default' \
':package -- The package to list the history for:_default' \
':repository -- The repository the package builds in:_default' \
':arch -- The architecture the package builds for:_default' \
&& ret=0
;;
(job-history)
_arguments "${_arguments_options[@]}" : \
'--package=[Only jobs for this package]:PACKAGE:_default' \
'--limit=[At most this many entries]:LIMIT:_default' \
'--code=[Only jobs with this result code (e.g. \`failed\`)]:CODE:_default' \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
':project -- The project the repository is in:_default' \
':repository -- The repository the jobs ran in:_default' \
':arch -- The architecture the jobs ran on:_default' \
&& ret=0
;;
(list-binaries)
_arguments "${_arguments_options[@]}" : \
'--package=[Only this package'\''s files; without it, the whole repository'\''s files]:PACKAGE:_default' \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
':project -- The project the package is in:_default' \
':repository -- The repository the files were built in:_default' \
':arch -- The architecture the files were built for:_default' \
&& ret=0
;;
(download-binary)
_arguments "${_arguments_options[@]}" : \
'--output=[Where to write the downloaded bytes\: the file path (it must not already exist), or the directory every built file goes into with \`--all\`]:OUTPUT:_files' \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'()--all[Download every built file into the \`--output\` directory; a file already there is skipped and reported, never re-downloaded and never deleted. The downloads are sequential, not atomic\: if one fails, the files already downloaded stay]' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
':project -- The project the package is in:_default' \
':package -- The package the file belongs to:_default' \
':repository -- The repository the file was built in:_default' \
':arch -- The architecture the file was built for:_default' \
'::filename -- The file to download (e.g. `foo-1.0-1.x86_64.rpm`). Required unless `--all` downloads every built file:_default' \
&& ret=0
;;
(fileinfo)
_arguments "${_arguments_options[@]}" : \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--ext[The \`fileinfo_ext\` view\: each dependency names the packages providing or requiring it]' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
':project -- The project the package is in:_default' \
':package -- The package the file belongs to:_default' \
':repository -- The repository the file was built in:_default' \
':arch -- The architecture the file was built for:_default' \
':filename -- The file to read the metadata for:_default' \
&& ret=0
;;
(dependson)
_arguments "${_arguments_options[@]}" : \
'--package=[Only this package'\''s dependencies]:PACKAGE:_default' \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
':project -- The project the repository is in:_default' \
':repository -- The repository to read:_default' \
':arch -- The architecture to read:_default' \
&& ret=0
;;
(whatdependson)
_arguments "${_arguments_options[@]}" : \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
':project -- The project the repository is in:_default' \
':package -- The package to find the dependents of:_default' \
':repository -- The repository to read:_default' \
':arch -- The architecture to read:_default' \
&& ret=0
;;
(check-constraints)
_arguments "${_arguments_options[@]}" : \
'--constraints=[The constraints document, as authored. Without it, an empty body is posted, which the server accepts]:CONSTRAINTS:_default' \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
':project -- The project the package is in:_default' \
':package -- The package to check the constraints for:_default' \
':repository -- The repository the package builds in:_default' \
':arch -- The architecture the package builds for:_default' \
&& ret=0
;;
(list-repositories)
_arguments "${_arguments_options[@]}" : \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
':project -- The project to read:_default' \
&& ret=0
;;
(help)
_arguments "${_arguments_options[@]}" : \
":: :_geectl__subcmd__build__subcmd__help_commands" \
"*::: :->help" \
&& ret=0

    case $state in
    (help)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:geectl-build-help-command-$line[1]:"
        case $line[1] in
            (results)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(wait-for-results)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(build-log)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(build-status)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(job-status)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(rebuild-package)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(restart-build)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(abort-build)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(wipe-binaries)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(buildinfo)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(buildconfig)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(build-history)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(job-history)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(list-binaries)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(download-binary)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(fileinfo)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(dependson)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(whatdependson)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(check-constraints)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(list-repositories)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(help)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
        esac
    ;;
esac
;;
        esac
    ;;
esac
;;
(local-build)
_arguments "${_arguments_options[@]}" : \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
":: :_geectl__subcmd__local-build_commands" \
"*::: :->local-build" \
&& ret=0

    case $state in
    (local-build)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:geectl-local-build-command-$line[1]:"
        case $line[1] in
            (service-run)
_arguments "${_arguments_options[@]}" : \
'--mode=[The run mode\: run, manualrun, precommit, localrun, disabledrun]:MODE:(run manualrun precommit localrun disabledrun)' \
'--service=[Run only this service, with the run mode forced as its mode]:SERVICE:_default' \
'--project=[The OBS project the services validate against\: in a git checkout it names the parent project, in an OBS checkout it defaults to the checkout'\''s project, and a value that disagrees with the checkout refuses]:PROJECT:_default' \
'--timeout-secs=[Timeout in seconds for the whole run]:TIMEOUT_SECS:_default' \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
':directory -- The source directory carrying the `_service` file:_files' \
&& ret=0
;;
(precommit)
_arguments "${_arguments_options[@]}" : \
'--project=[The OBS project the services validate against\: in a git checkout it names the parent project, in an OBS checkout it defaults to the checkout'\''s project, and a value that disagrees with the checkout refuses]:PROJECT:_default' \
'--timeout-secs=[Timeout in seconds for the whole gate]:TIMEOUT_SECS:_default' \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help (see more with '\''--help'\'')]' \
'--help[Print help (see more with '\''--help'\'')]' \
':directory -- The source directory carrying the `_service` file and optionally `pre_checkin.sh`:_files' \
&& ret=0
;;
(spec-cleaner)
_arguments "${_arguments_options[@]}" : \
'--timeout-secs=[Timeout in seconds for the run]:TIMEOUT_SECS:_default' \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
':directory -- The source directory carrying the spec:_files' \
':spec -- The spec file, inside the source directory:_files' \
'*::args -- Extra arguments for `spec-cleaner`, after `--`:_default' \
&& ret=0
;;
(build)
_arguments "${_arguments_options[@]}" : \
'--project=[The OBS project the package belongs to]:PROJECT:_default' \
'--repo=[The repository to build for (e.g. \`standard\`)]:REPO:_default' \
'--arch=[The architecture to build for; must equal the host arch]:ARCH:_default' \
'--flavor=[The multibuild flavor; required when the source carries a \`_multibuild\` file. One invocation builds one flavor]:FLAVOR:_default' \
'--release=[The release number, passed through to the engine]:RELEASE:_default' \
'--alternative-project=[Build against another project instead of \`project\`\: buildinfo, buildconfig, the trust check, and dep resolution use it. The root stays keyed by \`project\`]:ALTERNATIVE_PROJECT:_default' \
'--root=[Build in this directory instead of the derived cache-dir root]:ROOT:_files' \
'-k+[Save the built RPMs into this directory (created when missing) on a green build]:KEEP_RPMS:_files' \
'--keep-rpms=[Save the built RPMs into this directory (created when missing) on a green build]:KEEP_RPMS:_files' \
'-p+[Prefer RPMs from this directory when installing the build root\: a dep whose file is there is used instead of the cached or downloaded copy]:PREFER_PACKAGES:_files' \
'--prefer-packages=[Prefer RPMs from this directory when installing the build root\: a dep whose file is there is used instead of the cached or downloaded copy]:PREFER_PACKAGES:_files' \
'--timeout-secs=[Timeout in seconds for the whole run]:TIMEOUT_SECS:_default' \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--clean[Wipe the root before building]' \
'--trust-all-projects[Allow build dependencies from other users'\'' \`home\:\` projects (their packages run code in the build root). Without this such builds are refused]' \
'--refresh-image[Force an image pull even when the local image is fresh]' \
'--noinit[Skip the root initialization\: start the build immediately in the existing root. Needs an initialized root (a previous build wrote the rpmlist and buildconfig there)]' \
'--preinstall-image[Use the preinstall image when the buildinfo offers one. Off by default\: the engine extracts the image with bsdtar \`--chroot\` plus device nodes, which fail as a non-root user, so a rootless build never selects the image. Only set it when the engine runs as root]' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help (see more with '\''--help'\'')]' \
'--help[Print help (see more with '\''--help'\'')]' \
':directory -- The source directory (spec + sources):_files' \
&& ret=0
;;
(wipe)
_arguments "${_arguments_options[@]}" : \
'--project=[The OBS project the package belongs to. Required unless \`--root\` names the root directly]:PROJECT:_default' \
'--flavor=[The multibuild flavor; required when the source carries a \`_multibuild\` file]:FLAVOR:_default' \
'(--project --flavor)--root=[The root to wipe, named directly. Conflicts with the coordinates\: when set, the root is wiped as named]:ROOT:_files' \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
'::repo -- The repository the root was built for. Required unless `--root` names the root directly:_default' \
'::arch -- The architecture the root was built for. Required unless `--root` names the root directly:_default' \
'::spec -- The source directory (spec + sources); the package comes from the spec file in it, like the build. Required unless `--root` names the root directly:_files' \
&& ret=0
;;
(prune)
_arguments "${_arguments_options[@]}" : \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--apply[Delete what the plan lists]' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(periodic-install)
_arguments "${_arguments_options[@]}" : \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(periodic-uninstall)
_arguments "${_arguments_options[@]}" : \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(periodic-status)
_arguments "${_arguments_options[@]}" : \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(chroot)
_arguments "${_arguments_options[@]}" : \
'--project=[The OBS project the package belongs to]:PROJECT:_default' \
'--repo=[The repository the root was built for (e.g. \`standard\`)]:REPO:_default' \
'--arch=[The architecture the root was built for; defaults to the host arch]:ARCH:_default' \
'--flavor=[The multibuild flavor; required when the source carries a \`_multibuild\` file]:FLAVOR:_default' \
'--timeout-secs=[Timeout in seconds for the whole run]:TIMEOUT_SECS:_default' \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
':directory -- The source directory (spec + sources); the package comes from the spec file in it, like the build:_files' \
&& ret=0
;;
(log)
_arguments "${_arguments_options[@]}" : \
'--project=[The OBS project the package belongs to]:PROJECT:_default' \
'--flavor=[The multibuild flavor; required when the source carries a \`_multibuild\` file]:FLAVOR:_default' \
'--tail=[Print only the last N lines of the log]:TAIL:_default' \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--full[Read the whole log without the 4 MiB cap]' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
':repo -- The repository the root was built for:_default' \
':arch -- The architecture the root was built for:_default' \
':spec -- The source directory (spec + sources); the package comes from the spec file in it, like the build:_files' \
&& ret=0
;;
(help)
_arguments "${_arguments_options[@]}" : \
":: :_geectl__subcmd__local-build__subcmd__help_commands" \
"*::: :->help" \
&& ret=0

    case $state in
    (help)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:geectl-local-build-help-command-$line[1]:"
        case $line[1] in
            (service-run)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(precommit)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(spec-cleaner)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(build)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(wipe)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(prune)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(periodic-install)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(periodic-uninstall)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(periodic-status)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(chroot)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(log)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(help)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
        esac
    ;;
esac
;;
        esac
    ;;
esac
;;
(maintenance)
_arguments "${_arguments_options[@]}" : \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
":: :_geectl__subcmd__maintenance_commands" \
"*::: :->maintenance" \
&& ret=0

    case $state in
    (maintenance)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:geectl-maintenance-command-$line[1]:"
        case $line[1] in
            (maintained-instances)
_arguments "${_arguments_options[@]}" : \
'--package=[The package whose maintained instances to list]:PACKAGE:_default' \
'--attribute=[The attribute marking a project as maintaining the package (\`NAMESPACE\:NAME\`, default \`OBS\:Maintained\`)]:ATTRIBUTE:_default' \
'--update-project-attribute=[The attribute pointing a project at its update project (\`NAMESPACE\:NAME\`, default \`OBS\:UpdateProject\`)]:UPDATE_PROJECT_ATTRIBUTE:_default' \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(maintenance-branch)
_arguments "${_arguments_options[@]}" : \
'--package=[The package to branch into its maintenance branch project]:PACKAGE:_default' \
'--attribute=[The attribute marking a project as maintaining the package (\`NAMESPACE\:NAME\`, default \`OBS\:Maintained\`)]:ATTRIBUTE:_default' \
'--update-project-attribute=[The attribute pointing a project at its update project (\`NAMESPACE\:NAME\`, default \`OBS\:UpdateProject\`)]:UPDATE_PROJECT_ATTRIBUTE:_default' \
'--target-project=[Override the server-derived branch project (\`home\:<user>\:branches\:OBS_Maintained\:<package>\`)]:TARGET_PROJECT:_default' \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(help)
_arguments "${_arguments_options[@]}" : \
":: :_geectl__subcmd__maintenance__subcmd__help_commands" \
"*::: :->help" \
&& ret=0

    case $state in
    (help)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:geectl-maintenance-help-command-$line[1]:"
        case $line[1] in
            (maintained-instances)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(maintenance-branch)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(help)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
        esac
    ;;
esac
;;
        esac
    ;;
esac
;;
(service)
_arguments "${_arguments_options[@]}" : \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
":: :_geectl__subcmd__service_commands" \
"*::: :->service" \
&& ret=0

    case $state in
    (service)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:geectl-service-command-$line[1]:"
        case $line[1] in
            (list)
_arguments "${_arguments_options[@]}" : \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
':project -- The project the package lives in:_default' \
':package -- The package to list the services of:_default' \
&& ret=0
;;
(state)
_arguments "${_arguments_options[@]}" : \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
':project -- The project the package lives in:_default' \
':package -- The package to show the service state of:_default' \
&& ret=0
;;
(trigger)
_arguments "${_arguments_options[@]}" : \
'--service=[The service the trigger is for\: it must be defined in the package'\''s \`_service\`, and the trigger fails closed when it is not. The server runs the whole service set either way; without this flag, every service runs]:SERVICE:_default' \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
':project -- The project the package lives in:_default' \
':package -- The package to trigger the services of:_default' \
&& ret=0
;;
(help)
_arguments "${_arguments_options[@]}" : \
":: :_geectl__subcmd__service__subcmd__help_commands" \
"*::: :->help" \
&& ret=0

    case $state in
    (help)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:geectl-service-help-command-$line[1]:"
        case $line[1] in
            (list)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(state)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(trigger)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(help)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
        esac
    ;;
esac
;;
        esac
    ;;
esac
;;
(gitea)
_arguments "${_arguments_options[@]}" : \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
":: :_geectl__subcmd__gitea_commands" \
"*::: :->gitea" \
&& ret=0

    case $state in
    (gitea)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:geectl-gitea-command-$line[1]:"
        case $line[1] in
            (repo-get)
_arguments "${_arguments_options[@]}" : \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
':owner -- The repository owner:_default' \
':repo -- The repository name:_default' \
&& ret=0
;;
(repo-list)
_arguments "${_arguments_options[@]}" : \
'--owner=[List this user'\''s repositories instead of the token owner'\''s]:OWNER:_default' \
'--org=[List this organization'\''s repositories instead of (or, with \`--owner\`, after) the user'\''s]:ORG:_default' \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(repo-search)
_arguments "${_arguments_options[@]}" : \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
':query -- The search keyword:_default' \
&& ret=0
;;
(whoami)
_arguments "${_arguments_options[@]}" : \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(repo-create)
_arguments "${_arguments_options[@]}" : \
'--description=[A short description]:DESCRIPTION:_default' \
'--default-branch=[The default branch; the server default when unset]:DEFAULT_BRANCH:_default' \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--private[Create a private repository; public by default]' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
':name -- The repository name:_default' \
&& ret=0
;;
(repo-delete)
_arguments "${_arguments_options[@]}" : \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--force[Confirm the deletion]' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
':owner -- The repository owner:_default' \
':repo -- The repository name:_default' \
&& ret=0
;;
(repo-fork)
_arguments "${_arguments_options[@]}" : \
'--new-name=[Name for the fork; defaults to the source repo name]:NEW_NAME:_default' \
'--organization=[Organization to fork into; defaults to the token owner'\''s account]:ORGANIZATION:_default' \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
':owner -- The repository owner:_default' \
':repo -- The repository name:_default' \
&& ret=0
;;
(branch-list)
_arguments "${_arguments_options[@]}" : \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
':owner -- The repository owner:_default' \
':repo -- The repository name:_default' \
&& ret=0
;;
(branch-get)
_arguments "${_arguments_options[@]}" : \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
':owner -- The repository owner:_default' \
':repo -- The repository name:_default' \
':branch -- The branch name:_default' \
&& ret=0
;;
(branch-head)
_arguments "${_arguments_options[@]}" : \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
':owner -- The repository owner:_default' \
':repo -- The repository name:_default' \
':branch -- The branch name:_default' \
&& ret=0
;;
(raw-file)
_arguments "${_arguments_options[@]}" : \
'--ref=[The branch, tag, or commit to read from; the default branch when omitted]:GIT_REF:_default' \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
':owner -- The repository owner:_default' \
':repo -- The repository name:_default' \
':path -- The file path inside the repository:_default' \
&& ret=0
;;
(commit-statuses)
_arguments "${_arguments_options[@]}" : \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
':owner -- The repository owner:_default' \
':repo -- The repository name:_default' \
':sha -- The commit SHA:_default' \
&& ret=0
;;
(maintainers)
_arguments "${_arguments_options[@]}" : \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
':owner -- The repository owner:_default' \
':repo -- The repository name:_default' \
&& ret=0
;;
(cleanup-forks)
_arguments "${_arguments_options[@]}" : \
'--user=[The OBS user whose \`home\:<user>\:branches\:*\` projects are listed; defaults to the OBS credential username]:USER:_default' \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(help)
_arguments "${_arguments_options[@]}" : \
":: :_geectl__subcmd__gitea__subcmd__help_commands" \
"*::: :->help" \
&& ret=0

    case $state in
    (help)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:geectl-gitea-help-command-$line[1]:"
        case $line[1] in
            (repo-get)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(repo-list)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(repo-search)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(whoami)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(repo-create)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(repo-delete)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(repo-fork)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(branch-list)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(branch-get)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(branch-head)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(raw-file)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(commit-statuses)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(maintainers)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(cleanup-forks)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(help)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
        esac
    ;;
esac
;;
        esac
    ;;
esac
;;
(pull-request)
_arguments "${_arguments_options[@]}" : \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
":: :_geectl__subcmd__pull-request_commands" \
"*::: :->pull-request" \
&& ret=0

    case $state in
    (pull-request)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:geectl-pull-request-command-$line[1]:"
        case $line[1] in
            (show)
_arguments "${_arguments_options[@]}" : \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
':reference -- The pull-request handle\: `obs\:<id>` or `gitea\:<owner>/<repo>#<number>`:_default' \
&& ret=0
;;
(diff)
_arguments "${_arguments_options[@]}" : \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--xml[Render the OBS request XML with the diff injected, instead of the bare diff. Refused on a Gitea ref]' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
':reference -- The pull-request handle\: `obs\:<id>` or `gitea\:<owner>/<repo>#<number>`:_default' \
&& ret=0
;;
(search)
_arguments "${_arguments_options[@]}" : \
'--backend=[Only this backend (\`obs\` or \`gitea\`). With no backend both arms run, OBS first then Gitea]:BACKEND:(obs gitea)' \
'*--state=[Only pull requests in these unified states. With none given each arm applies its own default (new/review/declined on OBS, open on Gitea). OBS has no closed state\: \`--state closed\` reads OBS declined requests, which report as \`declined\`]:STATE:(open merged declined withdrawn closed superseded deleted)' \
'--creator=[Only pull requests by this author]:CREATOR:_default' \
'--reviewer=[Only pull requests with review requested of this login. Requires \`--backend gitea\`. Only the token owner'\''s login\: the server filters review requests for the authenticated user only]:REVIEWER:_default' \
'--repo=[Only pull requests in this Gitea repo (\`owner/repo\`). Requires \`--backend gitea\`]:REPO:_default' \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(approve)
_arguments "${_arguments_options[@]}" : \
'--message=[The message left with the review. Values starting with \`-\` are taken literally\: a message may open with a \`- \` bullet]:MESSAGE:_default' \
'--commit=[Pin the review to this head commit (Gitea only)\: the review is recorded against it server-side, and is refused when the head has moved. Best-effort\: the head is read just before the review posts, so a push landing between the read and the post is not caught. Refused on an OBS ref]:COMMIT:_default' \
'--user=[Act on the review asked of this user (OBS only)]:USER:_default' \
'--group=[Act on the review asked of this group (OBS only)]:GROUP:_default' \
'--project=[Act on the review asked of this project (OBS only)]:PROJECT:_default' \
'--package=[Act on the review asked of this package (OBS only; needs its project)]:PACKAGE:_default' \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
':reference -- The pull-request handle\: `obs\:<id>` or `gitea\:<owner>/<repo>#<number>`:_default' \
&& ret=0
;;
(request-changes)
_arguments "${_arguments_options[@]}" : \
'--message=[The message left with the review. Values starting with \`-\` are taken literally\: a message may open with a \`- \` bullet]:MESSAGE:_default' \
'--commit=[Pin the review to this head commit (Gitea only)\: the review is recorded against it server-side, and is refused when the head has moved. Best-effort\: the head is read just before the review posts, so a push landing between the read and the post is not caught. Refused on an OBS ref]:COMMIT:_default' \
'--user=[Act on the review asked of this user (OBS only)]:USER:_default' \
'--group=[Act on the review asked of this group (OBS only)]:GROUP:_default' \
'--project=[Act on the review asked of this project (OBS only)]:PROJECT:_default' \
'--package=[Act on the review asked of this package (OBS only; needs its project)]:PACKAGE:_default' \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
':reference -- The pull-request handle\: `obs\:<id>` or `gitea\:<owner>/<repo>#<number>`:_default' \
&& ret=0
;;
(comment)
_arguments "${_arguments_options[@]}" : \
'--body=[The comment text. Values starting with \`-\` are taken literally\: the body may open with a \`- \` bullet]:BODY:_default' \
'--parent=[The comment being replied to, when this is a reply (OBS only; refused on a Gitea ref)]:PARENT:_default' \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
':reference -- The pull-request handle\: `obs\:<id>` or `gitea\:<owner>/<repo>#<number>`:_default' \
&& ret=0
;;
(build-comment)
_arguments "${_arguments_options[@]}" : \
'--project=[The test project the build results are read from]:PROJECT:_default' \
'--package=[The package in the test project]:PACKAGE:_default' \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
':reference -- The pull-request handle\: `gitea\:<owner>/<repo>#<number>`. An OBS handle is refused\: the build link belongs on the Gitea pull request:_default' \
&& ret=0
;;
(comments)
_arguments "${_arguments_options[@]}" : \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
':reference -- The pull-request handle\: `obs\:<id>` or `gitea\:<owner>/<repo>#<number>`:_default' \
&& ret=0
;;
(cancel-review-request)
_arguments "${_arguments_options[@]}" : \
'*--reviewer=[The reviewers to cancel, repeatable; \`@team\` names a team]:REVIEWER:_default' \
'*--exclude=[With \`--all\`, leave these reviewers out; \`@team\` names a team]:EXCLUDE:_default' \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--all[Cancel every pending review request]' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
':reference -- The pull-request handle\: `obs\:<id>` or `gitea\:<owner>/<repo>#<number>`:_default' \
&& ret=0
;;
(merge)
_arguments "${_arguments_options[@]}" : \
'--message=[The message left with the merge. Values starting with \`-\` are taken literally\: a message may open with a \`- \` bullet]:MESSAGE:_default' \
'-m+[The changes entry for the forwarded request, overriding the original description on the OBS arm and the pull request'\''s text on the Gitea arm. Values starting with \`-\` are taken literally\: an entry may open with a \`- \` bullet]:FORWARD_MESSAGE:_default' \
'--forward-message=[The changes entry for the forwarded request, overriding the original description on the OBS arm and the pull request'\''s text on the Gitea arm. Values starting with \`-\` are taken literally\: an entry may open with a \`- \` bullet]:FORWARD_MESSAGE:_default' \
'--method=[The Gitea merge style (Gitea only; refused on an OBS ref)]:METHOD:(merge rebase rebase-merge squash fast-forward-only)' \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--force[Accept past open reviews (OBS only; refused on a Gitea ref)]' \
'--no-forward[Do not forward the pull request after merging it\: by default a merged Gitea pull request is forwarded to its link parent. (Gitea only; refused on an OBS ref.)]' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
':reference -- The pull-request handle\: `obs\:<id>` or `gitea\:<owner>/<repo>#<number>`:_default' \
&& ret=0
;;
(decline)
_arguments "${_arguments_options[@]}" : \
'--message=[The reason for the decline or withdrawal. Values starting with \`-\` are taken literally\: a message may open with a \`- \` bullet]:MESSAGE:_default' \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
':reference -- The pull-request handle\: `obs\:<id>` or `gitea\:<owner>/<repo>#<number>`:_default' \
&& ret=0
;;
(withdraw)
_arguments "${_arguments_options[@]}" : \
'--message=[The reason for the decline or withdrawal. Values starting with \`-\` are taken literally\: a message may open with a \`- \` bullet]:MESSAGE:_default' \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
':reference -- The pull-request handle\: `obs\:<id>` or `gitea\:<owner>/<repo>#<number>`:_default' \
&& ret=0
;;
(reopen)
_arguments "${_arguments_options[@]}" : \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
':reference -- The pull-request handle\: `obs\:<id>` or `gitea\:<owner>/<repo>#<number>`:_default' \
&& ret=0
;;
(submit)
_arguments "${_arguments_options[@]}" : \
'--target-project=[The project the request targets\: the devel project, or openSUSE\:Factory when the source is the recorded devel project and package. OBS only; required there]:TARGET_PROJECT:_default' \
'--target-package=[The package in the target project. OBS only; the checkout package when unset]:TARGET_PACKAGE:_default' \
'--description=[The request description\: one headline, then \`- \` lines for staging, needs, and rebuilds. OBS only; required there. Values starting with \`-\` are taken literally\: a description may open with a \`- \` bullet]:DESCRIPTION:_default' \
'--changes-entry=[The one \`.changes\` entry for this submission. It must name every CVE passed with \`--cve\`. OBS only; required there. Values starting with \`-\` are taken literally\: a \`.changes\` entry starts with \`- \`]:CHANGES_ENTRY:_default' \
'*--cve=[A CVE this submission claims to fix. Repeatable; each one must be named in \`--changes-entry\`. OBS only]:CVE:_default' \
'--repo=[The repo to open the pull request in (\`owner/repo\`). Gitea only; the parent of the checkout'\''s origin remote (the fork'\''s parent, never the fork) when unset]:REPO:_default' \
'--head=[The branch the change comes from. Gitea only; the checkout'\''s current branch when unset]:HEAD:_default' \
'--base=[The branch the pull request targets. Gitea only; the branch the checkout tracks (its scmsync branch) when unset, else the target repo'\''s default branch. The \`factory\` branch does not take pull requests]:BASE:_default' \
'--title=[The pull request title. Gitea only; required there. Values starting with \`-\` are taken literally]:TITLE:_default' \
'--body=[The pull request body. Gitea only. Values starting with \`-\` are taken literally\: the body may open with a \`- \` bullet]:BODY:_default' \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--without-cleanup[Keep the source branch on accept instead of deleting it (cleanup is the default). OBS only]' \
'--draft[Open the pull request as a draft. Gitea only]' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(cleanup)
_arguments "${_arguments_options[@]}" : \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(help)
_arguments "${_arguments_options[@]}" : \
":: :_geectl__subcmd__pull-request__subcmd__help_commands" \
"*::: :->help" \
&& ret=0

    case $state in
    (help)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:geectl-pull-request-help-command-$line[1]:"
        case $line[1] in
            (show)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(diff)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(search)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(approve)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(request-changes)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(comment)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(build-comment)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(comments)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(cancel-review-request)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(merge)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(decline)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(withdraw)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(reopen)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(submit)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(cleanup)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(help)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
        esac
    ;;
esac
;;
        esac
    ;;
esac
;;
(person)
_arguments "${_arguments_options[@]}" : \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
":: :_geectl__subcmd__person_commands" \
"*::: :->person" \
&& ret=0

    case $state in
    (person)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:geectl-person-command-$line[1]:"
        case $line[1] in
            (search)
_arguments "${_arguments_options[@]}" : \
'--backend=[Only search this backend (\`obs\` or \`gitea\`). With no backend both arms run, OBS first then Gitea]:BACKEND:(obs gitea)' \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
':query -- The query\: an OBS login, a Gitea keyword:_default' \
&& ret=0
;;
(help)
_arguments "${_arguments_options[@]}" : \
":: :_geectl__subcmd__person__subcmd__help_commands" \
"*::: :->help" \
&& ret=0

    case $state in
    (help)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:geectl-person-help-command-$line[1]:"
        case $line[1] in
            (search)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(help)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
        esac
    ;;
esac
;;
        esac
    ;;
esac
;;
(meta)
_arguments "${_arguments_options[@]}" : \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
":: :_geectl__subcmd__meta_commands" \
"*::: :->meta" \
&& ret=0

    case $state in
    (meta)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:geectl-meta-command-$line[1]:"
        case $line[1] in
            (set)
_arguments "${_arguments_options[@]}" : \
'(--repo)--project=[The OBS project whose meta to set]:PROJECT:_default' \
'--package=[The OBS package in \`--project\` whose meta to set; the project meta when omitted]:PACKAGE:_default' \
'(--project)--repo=[The Gitea repository, as \`owner/repo\`, whose settings to set]:REPO:_default' \
'--description=[A short description]:DESCRIPTION:_default' \
'--url=[A URL with more information; the repository website on the Gitea arm]:URL:_default' \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(help)
_arguments "${_arguments_options[@]}" : \
":: :_geectl__subcmd__meta__subcmd__help_commands" \
"*::: :->help" \
&& ret=0

    case $state in
    (help)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:geectl-meta-help-command-$line[1]:"
        case $line[1] in
            (set)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(help)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
        esac
    ;;
esac
;;
        esac
    ;;
esac
;;
(changelog)
_arguments "${_arguments_options[@]}" : \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
":: :_geectl__subcmd__changelog_commands" \
"*::: :->changelog" \
&& ret=0

    case $state in
    (changelog)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:geectl-changelog-command-$line[1]:"
        case $line[1] in
            (validate)
_arguments "${_arguments_options[@]}" : \
'--target-project=[The target project to compare against]:TARGET_PROJECT:_default' \
'--target-package=[The target package to compare against]:TARGET_PACKAGE:_default' \
'--entries=[Newest N entries to lint for format, and the number of new entries the submission may carry vs the target (default 1\: one entry per SR)]:ENTRIES:_default' \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
'::dir -- The package directory (default\: the current directory):_files' \
&& ret=0
;;
(generate)
_arguments "${_arguments_options[@]}" : \
'--target-project=[The target project to diff against]:TARGET_PROJECT:_default' \
'--target-package=[The target package to diff against]:TARGET_PACKAGE:_default' \
'--author=[The author line for the entry header, in \`Name <email>\` form — exactly what a bare \`changelog\` invocation writes]:AUTHOR:_default' \
'*-m+[Custom message bullets for the entry, one bullet per \`-m\`. Repeatable\: \`-m "Fix the crash" -m "Add the subcommand"\`. A message may span lines — embedded newlines become indented continuation lines of that bullet, and a blank line inside a message is a paragraph break within it. A leading \`- \`, \`* \` or \`+ \` marker is stripped, never double-bulleted. The message is the human detail\: it replaces the TODO summary slot, so \`--write\` no longer refuses the draft for the missing message; other TODO slots still refuse]:MESSAGE:_default' \
'*--message=[Custom message bullets for the entry, one bullet per \`-m\`. Repeatable\: \`-m "Fix the crash" -m "Add the subcommand"\`. A message may span lines — embedded newlines become indented continuation lines of that bullet, and a blank line inside a message is a paragraph break within it. A leading \`- \`, \`* \` or \`+ \` marker is stripped, never double-bulleted. The message is the human detail\: it replaces the TODO summary slot, so \`--write\` no longer refuses the draft for the missing message; other TODO slots still refuse]:MESSAGE:_default' \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--write[Prepend the draft into the package'\''s \`.changes\` file instead of printing it. The whole file is normalized on write\: trailing ASCII whitespace stripped, CRLF→LF, legacy encodings transcoded to UTF-8, a BOM stripped. The write is atomic and refuses a symlink destination, an unreadable file, or bytes no known encoding decodes. A file decoded from a guessed single-byte encoding is transcoded on the guess instead — a wrong guess cannot be detected — and the write warns about it, naming the encoding]' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
'::dir -- The package directory (default\: the current directory):_files' \
&& ret=0
;;
(identity)
_arguments "${_arguments_options[@]}" : \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
'::dir -- The package directory (default\: the current directory). The git level reads identity from here:_files' \
&& ret=0
;;
(help)
_arguments "${_arguments_options[@]}" : \
":: :_geectl__subcmd__changelog__subcmd__help_commands" \
"*::: :->help" \
&& ret=0

    case $state in
    (help)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:geectl-changelog-help-command-$line[1]:"
        case $line[1] in
            (validate)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(generate)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(identity)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(help)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
        esac
    ;;
esac
;;
        esac
    ;;
esac
;;
(checkout)
_arguments "${_arguments_options[@]}" : \
'--rev=[The revision to check out (a number or a source md5). The latest when omitted]:REV:_default' \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
':project -- The OBS project:_default' \
':package -- The OBS package:_default' \
&& ret=0
;;
(update)
_arguments "${_arguments_options[@]}" : \
'--rev=[The revision to update to (a number or a source md5). The latest when omitted]:REV:_default' \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(status)
_arguments "${_arguments_options[@]}" : \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(diff)
_arguments "${_arguments_options[@]}" : \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(add)
_arguments "${_arguments_options[@]}" : \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
'*::files -- The files to stage, relative to the checkout root:_default' \
&& ret=0
;;
(rm)
_arguments "${_arguments_options[@]}" : \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
'*::files -- The files to remove, relative to the checkout root:_default' \
&& ret=0
;;
(addremove)
_arguments "${_arguments_options[@]}" : \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(commit)
_arguments "${_arguments_options[@]}" : \
'-m+[The commit message]:MESSAGE:_default' \
'--message=[The commit message]:MESSAGE:_default' \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--noservice[Skip the service gate (source services and \`pre_checkin.sh\`)]' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(pull)
_arguments "${_arguments_options[@]}" : \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(resolve)
_arguments "${_arguments_options[@]}" : \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(prjconf)
_arguments "${_arguments_options[@]}" : \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
":: :_geectl__subcmd__prjconf_commands" \
"*::: :->prjconf" \
&& ret=0

    case $state in
    (prjconf)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:geectl-prjconf-command-$line[1]:"
        case $line[1] in
            (show)
_arguments "${_arguments_options[@]}" : \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help]' \
'--help[Print help]' \
':project -- The project to read the prjconf of:_default' \
&& ret=0
;;
(validate)
_arguments "${_arguments_options[@]}" : \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help (see more with '\''--help'\'')]' \
'--help[Print help (see more with '\''--help'\'')]' \
':project -- The project to validate the prjconf of:_default' \
&& ret=0
;;
(clean)
_arguments "${_arguments_options[@]}" : \
'--format=[Output format for the result payload. Applies to every command. Default\: the \`GEECTL_FORMAT\` environment variable, else the \`format\` key in the config file, else \`json\`]:FORMAT:(json text)' \
'--instance=[Select the instance for this call, by its alias in the config file'\''s \`\[instances\]\` table. The flag is authoritative\: it beats the \`GEECTL_OBS_URL\` environment variable, the configured URL, and any working-copy state. For OBS, the flagged alias'\''s URL replaces the configured URL and credentials resolve only from the flagged instance'\''s own variables (\`GEECTL_OBS_USERNAME_<ALIAS>\` / \`GEECTL_OBS_PASSWORD\[_FILE\]_<ALIAS>\` / \`GEECTL_OBS_APIKEY\[_FILE\]_<ALIAS>\`); another instance'\''s variables are never borrowed, and a missing credential for the flagged instance fails the call. For Gitea, the host is unaffected, but every Gitea call authenticates with the flagged alias'\''s \`GEECTL_GITEA_TOKEN\[_FILE\]_<ALIAS>\` only]:ALIAS:_default' \
'--include-path-dependent[Also remove lines proven dead only in this project'\''s own repository and architecture evaluations]' \
'--debug[Enable the debug channel\: human-readable diagnostic lines on stderr (HTTP attempts, retries, library decisions). Off by default; \`GEECTL_DEBUG=1\` enables it without the flag]' \
'--debug-body[Also print request and response bodies on the debug channel (capped at 2048 bytes, redacted, never headers). Implies \`--debug\`]' \
'--live[Actually perform mutations. Without it, only the plan is printed\: dry-run is the default for mutations]' \
'-h[Print help (see more with '\''--help'\'')]' \
'--help[Print help (see more with '\''--help'\'')]' \
':project -- The project to clean the prjconf of:_default' \
&& ret=0
;;
(help)
_arguments "${_arguments_options[@]}" : \
":: :_geectl__subcmd__prjconf__subcmd__help_commands" \
"*::: :->help" \
&& ret=0

    case $state in
    (help)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:geectl-prjconf-help-command-$line[1]:"
        case $line[1] in
            (show)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(validate)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(clean)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(help)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
        esac
    ;;
esac
;;
        esac
    ;;
esac
;;
(help)
_arguments "${_arguments_options[@]}" : \
":: :_geectl__subcmd__help_commands" \
"*::: :->help" \
&& ret=0

    case $state in
    (help)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:geectl-help-command-$line[1]:"
        case $line[1] in
            (version)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(config)
_arguments "${_arguments_options[@]}" : \
":: :_geectl__subcmd__help__subcmd__config_commands" \
"*::: :->config" \
&& ret=0

    case $state in
    (config)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:geectl-help-config-command-$line[1]:"
        case $line[1] in
            (show)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(init)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(import-oscrc)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
        esac
    ;;
esac
;;
(remote)
_arguments "${_arguments_options[@]}" : \
":: :_geectl__subcmd__help__subcmd__remote_commands" \
"*::: :->remote" \
&& ret=0

    case $state in
    (remote)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:geectl-help-remote-command-$line[1]:"
        case $line[1] in
            (cat)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(download)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(list-files)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(list-packages)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(list-projects)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(list-staging-projects)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(staging-project)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(package-log)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(revision-patch)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(project-meta)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(set-project-meta)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(package-meta)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(set-package-meta)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(update-package-meta-from-spec)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(search-packages)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(search-projects)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(search-projects-by-person)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(search-packages-by-person)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(count-scmsync-projects)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(list-scmsync-projects)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(owners)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(owners-for-binary)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(owners-for-package)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(maintainers)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(whois)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(link-target)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(project-diff)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(diff-packages)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(devel-project)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(repo-add)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(repo-remove)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(upload-file)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(create-project)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(create-package)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(create-token)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(delete-package)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(delete-link)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(delete-file)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(delete-project)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(undelete-package)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(undelete-project)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(aggregate-package)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(release-package)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(release-project)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(lock)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(unlock)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(add-channels)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(add-containers)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
        esac
    ;;
esac
;;
(request)
_arguments "${_arguments_options[@]}" : \
":: :_geectl__subcmd__help__subcmd__request_commands" \
"*::: :->request" \
&& ret=0

    case $state in
    (request)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:geectl-help-request-command-$line[1]:"
        case $line[1] in
            (show)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(log)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(search)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(list)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(diff)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(list-comments)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(accept)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(decline)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(revoke)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(change-state)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(try-change-state)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(add-review)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(change-review-state)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(add-comment)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(file)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(refile)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(supersede)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(file-add-role)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(file-change-devel)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(file-delete)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(file-maintenance-incident)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(refile-maintenance-incident)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
        esac
    ;;
esac
;;
(branch)
_arguments "${_arguments_options[@]}" : \
":: :_geectl__subcmd__help__subcmd__branch_commands" \
"*::: :->branch" \
&& ret=0

    case $state in
    (branch)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:geectl-help-branch-command-$line[1]:"
        case $line[1] in
            (create)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(branch-package)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(copy-package)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(copy-project)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(set-devel-project)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(link-package)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(repair-link)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(set-link-rev)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(set-link-rev-project)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(link-to-branch)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(detach-branch)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
        esac
    ;;
esac
;;
(build)
_arguments "${_arguments_options[@]}" : \
":: :_geectl__subcmd__help__subcmd__build_commands" \
"*::: :->build" \
&& ret=0

    case $state in
    (build)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:geectl-help-build-command-$line[1]:"
        case $line[1] in
            (results)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(wait-for-results)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(build-log)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(build-status)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(job-status)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(rebuild-package)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(restart-build)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(abort-build)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(wipe-binaries)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(buildinfo)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(buildconfig)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(build-history)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(job-history)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(list-binaries)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(download-binary)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(fileinfo)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(dependson)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(whatdependson)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(check-constraints)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(list-repositories)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
        esac
    ;;
esac
;;
(local-build)
_arguments "${_arguments_options[@]}" : \
":: :_geectl__subcmd__help__subcmd__local-build_commands" \
"*::: :->local-build" \
&& ret=0

    case $state in
    (local-build)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:geectl-help-local-build-command-$line[1]:"
        case $line[1] in
            (service-run)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(precommit)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(spec-cleaner)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(build)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(wipe)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(prune)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(periodic-install)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(periodic-uninstall)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(periodic-status)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(chroot)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(log)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
        esac
    ;;
esac
;;
(maintenance)
_arguments "${_arguments_options[@]}" : \
":: :_geectl__subcmd__help__subcmd__maintenance_commands" \
"*::: :->maintenance" \
&& ret=0

    case $state in
    (maintenance)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:geectl-help-maintenance-command-$line[1]:"
        case $line[1] in
            (maintained-instances)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(maintenance-branch)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
        esac
    ;;
esac
;;
(service)
_arguments "${_arguments_options[@]}" : \
":: :_geectl__subcmd__help__subcmd__service_commands" \
"*::: :->service" \
&& ret=0

    case $state in
    (service)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:geectl-help-service-command-$line[1]:"
        case $line[1] in
            (list)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(state)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(trigger)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
        esac
    ;;
esac
;;
(gitea)
_arguments "${_arguments_options[@]}" : \
":: :_geectl__subcmd__help__subcmd__gitea_commands" \
"*::: :->gitea" \
&& ret=0

    case $state in
    (gitea)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:geectl-help-gitea-command-$line[1]:"
        case $line[1] in
            (repo-get)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(repo-list)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(repo-search)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(whoami)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(repo-create)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(repo-delete)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(repo-fork)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(branch-list)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(branch-get)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(branch-head)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(raw-file)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(commit-statuses)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(maintainers)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(cleanup-forks)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
        esac
    ;;
esac
;;
(pull-request)
_arguments "${_arguments_options[@]}" : \
":: :_geectl__subcmd__help__subcmd__pull-request_commands" \
"*::: :->pull-request" \
&& ret=0

    case $state in
    (pull-request)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:geectl-help-pull-request-command-$line[1]:"
        case $line[1] in
            (show)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(diff)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(search)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(approve)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(request-changes)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(comment)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(build-comment)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(comments)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(cancel-review-request)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(merge)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(decline)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(withdraw)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(reopen)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(submit)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(cleanup)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
        esac
    ;;
esac
;;
(person)
_arguments "${_arguments_options[@]}" : \
":: :_geectl__subcmd__help__subcmd__person_commands" \
"*::: :->person" \
&& ret=0

    case $state in
    (person)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:geectl-help-person-command-$line[1]:"
        case $line[1] in
            (search)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
        esac
    ;;
esac
;;
(meta)
_arguments "${_arguments_options[@]}" : \
":: :_geectl__subcmd__help__subcmd__meta_commands" \
"*::: :->meta" \
&& ret=0

    case $state in
    (meta)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:geectl-help-meta-command-$line[1]:"
        case $line[1] in
            (set)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
        esac
    ;;
esac
;;
(changelog)
_arguments "${_arguments_options[@]}" : \
":: :_geectl__subcmd__help__subcmd__changelog_commands" \
"*::: :->changelog" \
&& ret=0

    case $state in
    (changelog)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:geectl-help-changelog-command-$line[1]:"
        case $line[1] in
            (validate)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(generate)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(identity)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
        esac
    ;;
esac
;;
(checkout)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(update)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(status)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(diff)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(add)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(rm)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(addremove)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(commit)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(pull)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(resolve)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(prjconf)
_arguments "${_arguments_options[@]}" : \
":: :_geectl__subcmd__help__subcmd__prjconf_commands" \
"*::: :->prjconf" \
&& ret=0

    case $state in
    (prjconf)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:geectl-help-prjconf-command-$line[1]:"
        case $line[1] in
            (show)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(validate)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(clean)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
        esac
    ;;
esac
;;
(help)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
        esac
    ;;
esac
;;
        esac
    ;;
esac
}

(( $+functions[_geectl_commands] )) ||
_geectl_commands() {
    local commands; commands=(
'version:Print version information as the result payload' \
'config:Inspect or initialise the config file' \
'remote:Remote reads and mutations\: inspect what the distribution holds, and change it' \
'request:Read and act on OBS submit requests, and file new ones' \
'branch:Branch, copy, and link packages' \
'build:Build-status commands\: results, logs, histories, binaries, buildconfig' \
'local-build:Local builds in the podman build vertical\: build a package, run its \`_service\` services, the precommit gate, \`spec-cleaner\`, wipe a build root, prune the cache, manage the daily periodic cleanup, enter the preserved build root, and read the log of the last local build' \
'maintenance:Maintenance operations\: maintained-instances listing and maintenance branching' \
'service:Service reads and trigger\: list a package'\''s services, their run state, and trigger a server-side run' \
'gitea:Gitea operations\: repository and branch reads and mutations on the configured instance' \
'pull-request:Read and act on pull requests\: OBS submit requests and Gitea pull requests as one surface' \
'person:Search for a person\: the OBS projects naming the login and the Gitea users matching the keyword, as one operation' \
'meta:Set the meta of an OBS project, an OBS package, or a Gitea repository as one operation\: the target selects the backend' \
'changelog:Validate and generate \`.changes\` entries in a package directory, pre-file' \
'checkout:Check out a package into a git working copy, under the current directory' \
'update:Update the working copy in the current directory to the latest base' \
'status:Show the working copy'\''s state against the base' \
'diff:Show the diff of the worktree and the index against the base, plus the link diff on a linked checkout' \
'add:Stage files for commit' \
'rm:Remove files from the working copy' \
'addremove:Stage new files and worktree deletions' \
'commit:Commit the staged changes' \
'pull:Pull a linked package to the link target'\''s latest revision' \
'resolve:Stage the resolved files and continue a stopped rebase' \
'prjconf:Read, validate and clean a project'\''s prjconf' \
'help:Print this message or the help of the given subcommand(s)' \
    )
    _describe -t commands 'geectl commands' commands "$@"
}
(( $+functions[_geectl__subcmd__add_commands] )) ||
_geectl__subcmd__add_commands() {
    local commands; commands=()
    _describe -t commands 'geectl add commands' commands "$@"
}
(( $+functions[_geectl__subcmd__addremove_commands] )) ||
_geectl__subcmd__addremove_commands() {
    local commands; commands=()
    _describe -t commands 'geectl addremove commands' commands "$@"
}
(( $+functions[_geectl__subcmd__branch_commands] )) ||
_geectl__subcmd__branch_commands() {
    local commands; commands=(
'create:Branch a git-backed package\: Gitea fork + OBS fork into a test project with the source project'\''s repositories. A mutation\: dry-run is the default (only the plan is printed); \`--live\` performs the branch' \
'branch-package:Branch a package\: create a linked copy under a branch project. A mutation\: dry-run is the default (only the plan is printed); \`--live\` performs the branch' \
'copy-package:Copy a package to another project (or another OBS instance with \`--to-apiurl\`). A mutation\: dry-run is the default' \
'copy-project:Copy a whole project. A mutation\: dry-run is the default' \
'set-devel-project:Set (or, without \`--devel-project\`, remove) the devel project of a package. A mutation\: dry-run is the default' \
'link-package:Link a package to another package. A mutation\: dry-run is the default' \
'repair-link:Repair a broken link\: strip the failing patches from \`_link\` server-side and pin it at a revision where it expands. A mutation\: dry-run is the default' \
'set-link-rev:Set the revision a link expands at. A mutation\: dry-run is the default' \
'set-link-rev-project:Set the revision every linked package in a project expands at. A mutation\: dry-run is the default' \
'link-to-branch:Turn a link into a branch\: replace the link with a copy of the linked files. A mutation\: dry-run is the default' \
'detach-branch:Detach a branch\: replace the branch link with a copy of the branched files. A mutation\: dry-run is the default' \
'help:Print this message or the help of the given subcommand(s)' \
    )
    _describe -t commands 'geectl branch commands' commands "$@"
}
(( $+functions[_geectl__subcmd__branch__subcmd__branch-package_commands] )) ||
_geectl__subcmd__branch__subcmd__branch-package_commands() {
    local commands; commands=()
    _describe -t commands 'geectl branch branch-package commands' commands "$@"
}
(( $+functions[_geectl__subcmd__branch__subcmd__copy-package_commands] )) ||
_geectl__subcmd__branch__subcmd__copy-package_commands() {
    local commands; commands=()
    _describe -t commands 'geectl branch copy-package commands' commands "$@"
}
(( $+functions[_geectl__subcmd__branch__subcmd__copy-project_commands] )) ||
_geectl__subcmd__branch__subcmd__copy-project_commands() {
    local commands; commands=()
    _describe -t commands 'geectl branch copy-project commands' commands "$@"
}
(( $+functions[_geectl__subcmd__branch__subcmd__create_commands] )) ||
_geectl__subcmd__branch__subcmd__create_commands() {
    local commands; commands=()
    _describe -t commands 'geectl branch create commands' commands "$@"
}
(( $+functions[_geectl__subcmd__branch__subcmd__detach-branch_commands] )) ||
_geectl__subcmd__branch__subcmd__detach-branch_commands() {
    local commands; commands=()
    _describe -t commands 'geectl branch detach-branch commands' commands "$@"
}
(( $+functions[_geectl__subcmd__branch__subcmd__help_commands] )) ||
_geectl__subcmd__branch__subcmd__help_commands() {
    local commands; commands=(
'create:Branch a git-backed package\: Gitea fork + OBS fork into a test project with the source project'\''s repositories. A mutation\: dry-run is the default (only the plan is printed); \`--live\` performs the branch' \
'branch-package:Branch a package\: create a linked copy under a branch project. A mutation\: dry-run is the default (only the plan is printed); \`--live\` performs the branch' \
'copy-package:Copy a package to another project (or another OBS instance with \`--to-apiurl\`). A mutation\: dry-run is the default' \
'copy-project:Copy a whole project. A mutation\: dry-run is the default' \
'set-devel-project:Set (or, without \`--devel-project\`, remove) the devel project of a package. A mutation\: dry-run is the default' \
'link-package:Link a package to another package. A mutation\: dry-run is the default' \
'repair-link:Repair a broken link\: strip the failing patches from \`_link\` server-side and pin it at a revision where it expands. A mutation\: dry-run is the default' \
'set-link-rev:Set the revision a link expands at. A mutation\: dry-run is the default' \
'set-link-rev-project:Set the revision every linked package in a project expands at. A mutation\: dry-run is the default' \
'link-to-branch:Turn a link into a branch\: replace the link with a copy of the linked files. A mutation\: dry-run is the default' \
'detach-branch:Detach a branch\: replace the branch link with a copy of the branched files. A mutation\: dry-run is the default' \
'help:Print this message or the help of the given subcommand(s)' \
    )
    _describe -t commands 'geectl branch help commands' commands "$@"
}
(( $+functions[_geectl__subcmd__branch__subcmd__help__subcmd__branch-package_commands] )) ||
_geectl__subcmd__branch__subcmd__help__subcmd__branch-package_commands() {
    local commands; commands=()
    _describe -t commands 'geectl branch help branch-package commands' commands "$@"
}
(( $+functions[_geectl__subcmd__branch__subcmd__help__subcmd__copy-package_commands] )) ||
_geectl__subcmd__branch__subcmd__help__subcmd__copy-package_commands() {
    local commands; commands=()
    _describe -t commands 'geectl branch help copy-package commands' commands "$@"
}
(( $+functions[_geectl__subcmd__branch__subcmd__help__subcmd__copy-project_commands] )) ||
_geectl__subcmd__branch__subcmd__help__subcmd__copy-project_commands() {
    local commands; commands=()
    _describe -t commands 'geectl branch help copy-project commands' commands "$@"
}
(( $+functions[_geectl__subcmd__branch__subcmd__help__subcmd__create_commands] )) ||
_geectl__subcmd__branch__subcmd__help__subcmd__create_commands() {
    local commands; commands=()
    _describe -t commands 'geectl branch help create commands' commands "$@"
}
(( $+functions[_geectl__subcmd__branch__subcmd__help__subcmd__detach-branch_commands] )) ||
_geectl__subcmd__branch__subcmd__help__subcmd__detach-branch_commands() {
    local commands; commands=()
    _describe -t commands 'geectl branch help detach-branch commands' commands "$@"
}
(( $+functions[_geectl__subcmd__branch__subcmd__help__subcmd__help_commands] )) ||
_geectl__subcmd__branch__subcmd__help__subcmd__help_commands() {
    local commands; commands=()
    _describe -t commands 'geectl branch help help commands' commands "$@"
}
(( $+functions[_geectl__subcmd__branch__subcmd__help__subcmd__link-package_commands] )) ||
_geectl__subcmd__branch__subcmd__help__subcmd__link-package_commands() {
    local commands; commands=()
    _describe -t commands 'geectl branch help link-package commands' commands "$@"
}
(( $+functions[_geectl__subcmd__branch__subcmd__help__subcmd__link-to-branch_commands] )) ||
_geectl__subcmd__branch__subcmd__help__subcmd__link-to-branch_commands() {
    local commands; commands=()
    _describe -t commands 'geectl branch help link-to-branch commands' commands "$@"
}
(( $+functions[_geectl__subcmd__branch__subcmd__help__subcmd__repair-link_commands] )) ||
_geectl__subcmd__branch__subcmd__help__subcmd__repair-link_commands() {
    local commands; commands=()
    _describe -t commands 'geectl branch help repair-link commands' commands "$@"
}
(( $+functions[_geectl__subcmd__branch__subcmd__help__subcmd__set-devel-project_commands] )) ||
_geectl__subcmd__branch__subcmd__help__subcmd__set-devel-project_commands() {
    local commands; commands=()
    _describe -t commands 'geectl branch help set-devel-project commands' commands "$@"
}
(( $+functions[_geectl__subcmd__branch__subcmd__help__subcmd__set-link-rev_commands] )) ||
_geectl__subcmd__branch__subcmd__help__subcmd__set-link-rev_commands() {
    local commands; commands=()
    _describe -t commands 'geectl branch help set-link-rev commands' commands "$@"
}
(( $+functions[_geectl__subcmd__branch__subcmd__help__subcmd__set-link-rev-project_commands] )) ||
_geectl__subcmd__branch__subcmd__help__subcmd__set-link-rev-project_commands() {
    local commands; commands=()
    _describe -t commands 'geectl branch help set-link-rev-project commands' commands "$@"
}
(( $+functions[_geectl__subcmd__branch__subcmd__link-package_commands] )) ||
_geectl__subcmd__branch__subcmd__link-package_commands() {
    local commands; commands=()
    _describe -t commands 'geectl branch link-package commands' commands "$@"
}
(( $+functions[_geectl__subcmd__branch__subcmd__link-to-branch_commands] )) ||
_geectl__subcmd__branch__subcmd__link-to-branch_commands() {
    local commands; commands=()
    _describe -t commands 'geectl branch link-to-branch commands' commands "$@"
}
(( $+functions[_geectl__subcmd__branch__subcmd__repair-link_commands] )) ||
_geectl__subcmd__branch__subcmd__repair-link_commands() {
    local commands; commands=()
    _describe -t commands 'geectl branch repair-link commands' commands "$@"
}
(( $+functions[_geectl__subcmd__branch__subcmd__set-devel-project_commands] )) ||
_geectl__subcmd__branch__subcmd__set-devel-project_commands() {
    local commands; commands=()
    _describe -t commands 'geectl branch set-devel-project commands' commands "$@"
}
(( $+functions[_geectl__subcmd__branch__subcmd__set-link-rev_commands] )) ||
_geectl__subcmd__branch__subcmd__set-link-rev_commands() {
    local commands; commands=()
    _describe -t commands 'geectl branch set-link-rev commands' commands "$@"
}
(( $+functions[_geectl__subcmd__branch__subcmd__set-link-rev-project_commands] )) ||
_geectl__subcmd__branch__subcmd__set-link-rev-project_commands() {
    local commands; commands=()
    _describe -t commands 'geectl branch set-link-rev-project commands' commands "$@"
}
(( $+functions[_geectl__subcmd__build_commands] )) ||
_geectl__subcmd__build_commands() {
    local commands; commands=(
'results:Read the build-results matrix for a project' \
'wait-for-results:Poll the build results until they settle, or the deadline passes' \
'build-log:Read the build log for a package' \
'build-status:Read the build status code for a package' \
'job-status:Read the running build job for a package, if any' \
'rebuild-package:Rebuild a package\: rerun a build without touching the sources. A mutation\: dry-run is the default (only the plan is printed); \`--live\` performs the rebuild' \
'restart-build:Restart the running build of a package, without touching the sources. A mutation\: dry-run is the default (only the plan is printed); \`--live\` performs the restart' \
'abort-build:Abort the running build of a package. A mutation\: dry-run is the default (only the plan is printed); \`--live\` performs the abort' \
'wipe-binaries:Wipe the binary packages of a project or a package. A mutation\: dry-run is the default (only the plan is printed); \`--live\` performs the wipe. At least one scope selector is required\: \`--all\`, \`--arch\`, \`--repository\`, or a status filter' \
'buildinfo:Read the buildinfo for a package\: the blocking error, if any, and the dependencies the build pulls in' \
'buildconfig:Read the build configuration for a project/repository\: the expanded \`_buildconfig\` text the engine takes as \`--dist\`' \
'build-history:List the completed builds for a package, newest first' \
'job-history:List the build jobs that ran in a repository, newest first' \
'list-binaries:List the built files for a package (or the whole repository)' \
'download-binary:Download one built file by name. With \`--all\`, every built file' \
'fileinfo:Read the metadata of a built file\: RPM header fields plus dependency lists' \
'dependson:List which packages each package depends on for its build' \
'whatdependson:List the packages that depend on a package for their build' \
'check-constraints:List the worker ids satisfying a \`_constraints\` document' \
'list-repositories:List the repositories of a project' \
'help:Print this message or the help of the given subcommand(s)' \
    )
    _describe -t commands 'geectl build commands' commands "$@"
}
(( $+functions[_geectl__subcmd__build__subcmd__abort-build_commands] )) ||
_geectl__subcmd__build__subcmd__abort-build_commands() {
    local commands; commands=()
    _describe -t commands 'geectl build abort-build commands' commands "$@"
}
(( $+functions[_geectl__subcmd__build__subcmd__build-history_commands] )) ||
_geectl__subcmd__build__subcmd__build-history_commands() {
    local commands; commands=()
    _describe -t commands 'geectl build build-history commands' commands "$@"
}
(( $+functions[_geectl__subcmd__build__subcmd__build-log_commands] )) ||
_geectl__subcmd__build__subcmd__build-log_commands() {
    local commands; commands=()
    _describe -t commands 'geectl build build-log commands' commands "$@"
}
(( $+functions[_geectl__subcmd__build__subcmd__build-status_commands] )) ||
_geectl__subcmd__build__subcmd__build-status_commands() {
    local commands; commands=()
    _describe -t commands 'geectl build build-status commands' commands "$@"
}
(( $+functions[_geectl__subcmd__build__subcmd__buildconfig_commands] )) ||
_geectl__subcmd__build__subcmd__buildconfig_commands() {
    local commands; commands=()
    _describe -t commands 'geectl build buildconfig commands' commands "$@"
}
(( $+functions[_geectl__subcmd__build__subcmd__buildinfo_commands] )) ||
_geectl__subcmd__build__subcmd__buildinfo_commands() {
    local commands; commands=()
    _describe -t commands 'geectl build buildinfo commands' commands "$@"
}
(( $+functions[_geectl__subcmd__build__subcmd__check-constraints_commands] )) ||
_geectl__subcmd__build__subcmd__check-constraints_commands() {
    local commands; commands=()
    _describe -t commands 'geectl build check-constraints commands' commands "$@"
}
(( $+functions[_geectl__subcmd__build__subcmd__dependson_commands] )) ||
_geectl__subcmd__build__subcmd__dependson_commands() {
    local commands; commands=()
    _describe -t commands 'geectl build dependson commands' commands "$@"
}
(( $+functions[_geectl__subcmd__build__subcmd__download-binary_commands] )) ||
_geectl__subcmd__build__subcmd__download-binary_commands() {
    local commands; commands=()
    _describe -t commands 'geectl build download-binary commands' commands "$@"
}
(( $+functions[_geectl__subcmd__build__subcmd__fileinfo_commands] )) ||
_geectl__subcmd__build__subcmd__fileinfo_commands() {
    local commands; commands=()
    _describe -t commands 'geectl build fileinfo commands' commands "$@"
}
(( $+functions[_geectl__subcmd__build__subcmd__help_commands] )) ||
_geectl__subcmd__build__subcmd__help_commands() {
    local commands; commands=(
'results:Read the build-results matrix for a project' \
'wait-for-results:Poll the build results until they settle, or the deadline passes' \
'build-log:Read the build log for a package' \
'build-status:Read the build status code for a package' \
'job-status:Read the running build job for a package, if any' \
'rebuild-package:Rebuild a package\: rerun a build without touching the sources. A mutation\: dry-run is the default (only the plan is printed); \`--live\` performs the rebuild' \
'restart-build:Restart the running build of a package, without touching the sources. A mutation\: dry-run is the default (only the plan is printed); \`--live\` performs the restart' \
'abort-build:Abort the running build of a package. A mutation\: dry-run is the default (only the plan is printed); \`--live\` performs the abort' \
'wipe-binaries:Wipe the binary packages of a project or a package. A mutation\: dry-run is the default (only the plan is printed); \`--live\` performs the wipe. At least one scope selector is required\: \`--all\`, \`--arch\`, \`--repository\`, or a status filter' \
'buildinfo:Read the buildinfo for a package\: the blocking error, if any, and the dependencies the build pulls in' \
'buildconfig:Read the build configuration for a project/repository\: the expanded \`_buildconfig\` text the engine takes as \`--dist\`' \
'build-history:List the completed builds for a package, newest first' \
'job-history:List the build jobs that ran in a repository, newest first' \
'list-binaries:List the built files for a package (or the whole repository)' \
'download-binary:Download one built file by name. With \`--all\`, every built file' \
'fileinfo:Read the metadata of a built file\: RPM header fields plus dependency lists' \
'dependson:List which packages each package depends on for its build' \
'whatdependson:List the packages that depend on a package for their build' \
'check-constraints:List the worker ids satisfying a \`_constraints\` document' \
'list-repositories:List the repositories of a project' \
'help:Print this message or the help of the given subcommand(s)' \
    )
    _describe -t commands 'geectl build help commands' commands "$@"
}
(( $+functions[_geectl__subcmd__build__subcmd__help__subcmd__abort-build_commands] )) ||
_geectl__subcmd__build__subcmd__help__subcmd__abort-build_commands() {
    local commands; commands=()
    _describe -t commands 'geectl build help abort-build commands' commands "$@"
}
(( $+functions[_geectl__subcmd__build__subcmd__help__subcmd__build-history_commands] )) ||
_geectl__subcmd__build__subcmd__help__subcmd__build-history_commands() {
    local commands; commands=()
    _describe -t commands 'geectl build help build-history commands' commands "$@"
}
(( $+functions[_geectl__subcmd__build__subcmd__help__subcmd__build-log_commands] )) ||
_geectl__subcmd__build__subcmd__help__subcmd__build-log_commands() {
    local commands; commands=()
    _describe -t commands 'geectl build help build-log commands' commands "$@"
}
(( $+functions[_geectl__subcmd__build__subcmd__help__subcmd__build-status_commands] )) ||
_geectl__subcmd__build__subcmd__help__subcmd__build-status_commands() {
    local commands; commands=()
    _describe -t commands 'geectl build help build-status commands' commands "$@"
}
(( $+functions[_geectl__subcmd__build__subcmd__help__subcmd__buildconfig_commands] )) ||
_geectl__subcmd__build__subcmd__help__subcmd__buildconfig_commands() {
    local commands; commands=()
    _describe -t commands 'geectl build help buildconfig commands' commands "$@"
}
(( $+functions[_geectl__subcmd__build__subcmd__help__subcmd__buildinfo_commands] )) ||
_geectl__subcmd__build__subcmd__help__subcmd__buildinfo_commands() {
    local commands; commands=()
    _describe -t commands 'geectl build help buildinfo commands' commands "$@"
}
(( $+functions[_geectl__subcmd__build__subcmd__help__subcmd__check-constraints_commands] )) ||
_geectl__subcmd__build__subcmd__help__subcmd__check-constraints_commands() {
    local commands; commands=()
    _describe -t commands 'geectl build help check-constraints commands' commands "$@"
}
(( $+functions[_geectl__subcmd__build__subcmd__help__subcmd__dependson_commands] )) ||
_geectl__subcmd__build__subcmd__help__subcmd__dependson_commands() {
    local commands; commands=()
    _describe -t commands 'geectl build help dependson commands' commands "$@"
}
(( $+functions[_geectl__subcmd__build__subcmd__help__subcmd__download-binary_commands] )) ||
_geectl__subcmd__build__subcmd__help__subcmd__download-binary_commands() {
    local commands; commands=()
    _describe -t commands 'geectl build help download-binary commands' commands "$@"
}
(( $+functions[_geectl__subcmd__build__subcmd__help__subcmd__fileinfo_commands] )) ||
_geectl__subcmd__build__subcmd__help__subcmd__fileinfo_commands() {
    local commands; commands=()
    _describe -t commands 'geectl build help fileinfo commands' commands "$@"
}
(( $+functions[_geectl__subcmd__build__subcmd__help__subcmd__help_commands] )) ||
_geectl__subcmd__build__subcmd__help__subcmd__help_commands() {
    local commands; commands=()
    _describe -t commands 'geectl build help help commands' commands "$@"
}
(( $+functions[_geectl__subcmd__build__subcmd__help__subcmd__job-history_commands] )) ||
_geectl__subcmd__build__subcmd__help__subcmd__job-history_commands() {
    local commands; commands=()
    _describe -t commands 'geectl build help job-history commands' commands "$@"
}
(( $+functions[_geectl__subcmd__build__subcmd__help__subcmd__job-status_commands] )) ||
_geectl__subcmd__build__subcmd__help__subcmd__job-status_commands() {
    local commands; commands=()
    _describe -t commands 'geectl build help job-status commands' commands "$@"
}
(( $+functions[_geectl__subcmd__build__subcmd__help__subcmd__list-binaries_commands] )) ||
_geectl__subcmd__build__subcmd__help__subcmd__list-binaries_commands() {
    local commands; commands=()
    _describe -t commands 'geectl build help list-binaries commands' commands "$@"
}
(( $+functions[_geectl__subcmd__build__subcmd__help__subcmd__list-repositories_commands] )) ||
_geectl__subcmd__build__subcmd__help__subcmd__list-repositories_commands() {
    local commands; commands=()
    _describe -t commands 'geectl build help list-repositories commands' commands "$@"
}
(( $+functions[_geectl__subcmd__build__subcmd__help__subcmd__rebuild-package_commands] )) ||
_geectl__subcmd__build__subcmd__help__subcmd__rebuild-package_commands() {
    local commands; commands=()
    _describe -t commands 'geectl build help rebuild-package commands' commands "$@"
}
(( $+functions[_geectl__subcmd__build__subcmd__help__subcmd__restart-build_commands] )) ||
_geectl__subcmd__build__subcmd__help__subcmd__restart-build_commands() {
    local commands; commands=()
    _describe -t commands 'geectl build help restart-build commands' commands "$@"
}
(( $+functions[_geectl__subcmd__build__subcmd__help__subcmd__results_commands] )) ||
_geectl__subcmd__build__subcmd__help__subcmd__results_commands() {
    local commands; commands=()
    _describe -t commands 'geectl build help results commands' commands "$@"
}
(( $+functions[_geectl__subcmd__build__subcmd__help__subcmd__wait-for-results_commands] )) ||
_geectl__subcmd__build__subcmd__help__subcmd__wait-for-results_commands() {
    local commands; commands=()
    _describe -t commands 'geectl build help wait-for-results commands' commands "$@"
}
(( $+functions[_geectl__subcmd__build__subcmd__help__subcmd__whatdependson_commands] )) ||
_geectl__subcmd__build__subcmd__help__subcmd__whatdependson_commands() {
    local commands; commands=()
    _describe -t commands 'geectl build help whatdependson commands' commands "$@"
}
(( $+functions[_geectl__subcmd__build__subcmd__help__subcmd__wipe-binaries_commands] )) ||
_geectl__subcmd__build__subcmd__help__subcmd__wipe-binaries_commands() {
    local commands; commands=()
    _describe -t commands 'geectl build help wipe-binaries commands' commands "$@"
}
(( $+functions[_geectl__subcmd__build__subcmd__job-history_commands] )) ||
_geectl__subcmd__build__subcmd__job-history_commands() {
    local commands; commands=()
    _describe -t commands 'geectl build job-history commands' commands "$@"
}
(( $+functions[_geectl__subcmd__build__subcmd__job-status_commands] )) ||
_geectl__subcmd__build__subcmd__job-status_commands() {
    local commands; commands=()
    _describe -t commands 'geectl build job-status commands' commands "$@"
}
(( $+functions[_geectl__subcmd__build__subcmd__list-binaries_commands] )) ||
_geectl__subcmd__build__subcmd__list-binaries_commands() {
    local commands; commands=()
    _describe -t commands 'geectl build list-binaries commands' commands "$@"
}
(( $+functions[_geectl__subcmd__build__subcmd__list-repositories_commands] )) ||
_geectl__subcmd__build__subcmd__list-repositories_commands() {
    local commands; commands=()
    _describe -t commands 'geectl build list-repositories commands' commands "$@"
}
(( $+functions[_geectl__subcmd__build__subcmd__rebuild-package_commands] )) ||
_geectl__subcmd__build__subcmd__rebuild-package_commands() {
    local commands; commands=()
    _describe -t commands 'geectl build rebuild-package commands' commands "$@"
}
(( $+functions[_geectl__subcmd__build__subcmd__restart-build_commands] )) ||
_geectl__subcmd__build__subcmd__restart-build_commands() {
    local commands; commands=()
    _describe -t commands 'geectl build restart-build commands' commands "$@"
}
(( $+functions[_geectl__subcmd__build__subcmd__results_commands] )) ||
_geectl__subcmd__build__subcmd__results_commands() {
    local commands; commands=()
    _describe -t commands 'geectl build results commands' commands "$@"
}
(( $+functions[_geectl__subcmd__build__subcmd__wait-for-results_commands] )) ||
_geectl__subcmd__build__subcmd__wait-for-results_commands() {
    local commands; commands=()
    _describe -t commands 'geectl build wait-for-results commands' commands "$@"
}
(( $+functions[_geectl__subcmd__build__subcmd__whatdependson_commands] )) ||
_geectl__subcmd__build__subcmd__whatdependson_commands() {
    local commands; commands=()
    _describe -t commands 'geectl build whatdependson commands' commands "$@"
}
(( $+functions[_geectl__subcmd__build__subcmd__wipe-binaries_commands] )) ||
_geectl__subcmd__build__subcmd__wipe-binaries_commands() {
    local commands; commands=()
    _describe -t commands 'geectl build wipe-binaries commands' commands "$@"
}
(( $+functions[_geectl__subcmd__changelog_commands] )) ||
_geectl__subcmd__changelog_commands() {
    local commands; commands=(
'validate:Validate the \`.changes\` entries in a package directory against the V1–V12 rules. Local-only by default; with \`--target-project\`/\`--target-package\` the target-relative rules (V2, V4, V5 newest-vs-target, V11 older entries) run against the target'\''s current files. Exits non-zero naming the failing rule' \
'generate:Generate a skeleton \`.changes\` entry from the diff against the target (version line, patch and dependency lists, license lines) plus the upstream NEWS section for the new version, extracted from the source tarball. The output is a draft\: what the diff cannot supply arrives as explicit \`TODO\` slots for a human to fill. \`--write\` prepends the draft into the package'\''s \`.changes\` file, and refuses an incomplete draft (V9 needs-detail)' \
'identity:Print the resolved \`.changes\` entry author in \`Name <email>\` form — the same value a bare \`changelog generate\` would write. The author resolves through the identity chain (#405)\: config \`\[identity\]\`, then \`VC_REALNAME\` / \`VC_MAILADDR\`, then the OBS account, then git identity; the command refuses when none yields an identity' \
'help:Print this message or the help of the given subcommand(s)' \
    )
    _describe -t commands 'geectl changelog commands' commands "$@"
}
(( $+functions[_geectl__subcmd__changelog__subcmd__generate_commands] )) ||
_geectl__subcmd__changelog__subcmd__generate_commands() {
    local commands; commands=()
    _describe -t commands 'geectl changelog generate commands' commands "$@"
}
(( $+functions[_geectl__subcmd__changelog__subcmd__help_commands] )) ||
_geectl__subcmd__changelog__subcmd__help_commands() {
    local commands; commands=(
'validate:Validate the \`.changes\` entries in a package directory against the V1–V12 rules. Local-only by default; with \`--target-project\`/\`--target-package\` the target-relative rules (V2, V4, V5 newest-vs-target, V11 older entries) run against the target'\''s current files. Exits non-zero naming the failing rule' \
'generate:Generate a skeleton \`.changes\` entry from the diff against the target (version line, patch and dependency lists, license lines) plus the upstream NEWS section for the new version, extracted from the source tarball. The output is a draft\: what the diff cannot supply arrives as explicit \`TODO\` slots for a human to fill. \`--write\` prepends the draft into the package'\''s \`.changes\` file, and refuses an incomplete draft (V9 needs-detail)' \
'identity:Print the resolved \`.changes\` entry author in \`Name <email>\` form — the same value a bare \`changelog generate\` would write. The author resolves through the identity chain (#405)\: config \`\[identity\]\`, then \`VC_REALNAME\` / \`VC_MAILADDR\`, then the OBS account, then git identity; the command refuses when none yields an identity' \
'help:Print this message or the help of the given subcommand(s)' \
    )
    _describe -t commands 'geectl changelog help commands' commands "$@"
}
(( $+functions[_geectl__subcmd__changelog__subcmd__help__subcmd__generate_commands] )) ||
_geectl__subcmd__changelog__subcmd__help__subcmd__generate_commands() {
    local commands; commands=()
    _describe -t commands 'geectl changelog help generate commands' commands "$@"
}
(( $+functions[_geectl__subcmd__changelog__subcmd__help__subcmd__help_commands] )) ||
_geectl__subcmd__changelog__subcmd__help__subcmd__help_commands() {
    local commands; commands=()
    _describe -t commands 'geectl changelog help help commands' commands "$@"
}
(( $+functions[_geectl__subcmd__changelog__subcmd__help__subcmd__identity_commands] )) ||
_geectl__subcmd__changelog__subcmd__help__subcmd__identity_commands() {
    local commands; commands=()
    _describe -t commands 'geectl changelog help identity commands' commands "$@"
}
(( $+functions[_geectl__subcmd__changelog__subcmd__help__subcmd__validate_commands] )) ||
_geectl__subcmd__changelog__subcmd__help__subcmd__validate_commands() {
    local commands; commands=()
    _describe -t commands 'geectl changelog help validate commands' commands "$@"
}
(( $+functions[_geectl__subcmd__changelog__subcmd__identity_commands] )) ||
_geectl__subcmd__changelog__subcmd__identity_commands() {
    local commands; commands=()
    _describe -t commands 'geectl changelog identity commands' commands "$@"
}
(( $+functions[_geectl__subcmd__changelog__subcmd__validate_commands] )) ||
_geectl__subcmd__changelog__subcmd__validate_commands() {
    local commands; commands=()
    _describe -t commands 'geectl changelog validate commands' commands "$@"
}
(( $+functions[_geectl__subcmd__checkout_commands] )) ||
_geectl__subcmd__checkout_commands() {
    local commands; commands=()
    _describe -t commands 'geectl checkout commands' commands "$@"
}
(( $+functions[_geectl__subcmd__commit_commands] )) ||
_geectl__subcmd__commit_commands() {
    local commands; commands=()
    _describe -t commands 'geectl commit commands' commands "$@"
}
(( $+functions[_geectl__subcmd__config_commands] )) ||
_geectl__subcmd__config_commands() {
    local commands; commands=(
'show:Show the effective config\: endpoint URLs with their source, and credential status (never a secret value)' \
'init:Write the config template (endpoints only). Refuses if the file already exists' \
'import-oscrc:Import one instance mapping from an oscrc file (one-time migration). Reads the oscrc file once, on explicit request — geectl never reads it at runtime — and writes only the alias to apiurl mapping into the config \`\[instances\]\` table. The password is never stored and never printed\: set it with the printed \`GEECTL_OBS_PASSWORD_<ALIAS>\` variable yourself. The username is not written into the config either\: it stays in the environment as \`GEECTL_OBS_USERNAME_<ALIAS>\`. Dry-run is the default (only the plan is printed); \`--live\` writes the config' \
'help:Print this message or the help of the given subcommand(s)' \
    )
    _describe -t commands 'geectl config commands' commands "$@"
}
(( $+functions[_geectl__subcmd__config__subcmd__help_commands] )) ||
_geectl__subcmd__config__subcmd__help_commands() {
    local commands; commands=(
'show:Show the effective config\: endpoint URLs with their source, and credential status (never a secret value)' \
'init:Write the config template (endpoints only). Refuses if the file already exists' \
'import-oscrc:Import one instance mapping from an oscrc file (one-time migration). Reads the oscrc file once, on explicit request — geectl never reads it at runtime — and writes only the alias to apiurl mapping into the config \`\[instances\]\` table. The password is never stored and never printed\: set it with the printed \`GEECTL_OBS_PASSWORD_<ALIAS>\` variable yourself. The username is not written into the config either\: it stays in the environment as \`GEECTL_OBS_USERNAME_<ALIAS>\`. Dry-run is the default (only the plan is printed); \`--live\` writes the config' \
'help:Print this message or the help of the given subcommand(s)' \
    )
    _describe -t commands 'geectl config help commands' commands "$@"
}
(( $+functions[_geectl__subcmd__config__subcmd__help__subcmd__help_commands] )) ||
_geectl__subcmd__config__subcmd__help__subcmd__help_commands() {
    local commands; commands=()
    _describe -t commands 'geectl config help help commands' commands "$@"
}
(( $+functions[_geectl__subcmd__config__subcmd__help__subcmd__import-oscrc_commands] )) ||
_geectl__subcmd__config__subcmd__help__subcmd__import-oscrc_commands() {
    local commands; commands=()
    _describe -t commands 'geectl config help import-oscrc commands' commands "$@"
}
(( $+functions[_geectl__subcmd__config__subcmd__help__subcmd__init_commands] )) ||
_geectl__subcmd__config__subcmd__help__subcmd__init_commands() {
    local commands; commands=()
    _describe -t commands 'geectl config help init commands' commands "$@"
}
(( $+functions[_geectl__subcmd__config__subcmd__help__subcmd__show_commands] )) ||
_geectl__subcmd__config__subcmd__help__subcmd__show_commands() {
    local commands; commands=()
    _describe -t commands 'geectl config help show commands' commands "$@"
}
(( $+functions[_geectl__subcmd__config__subcmd__import-oscrc_commands] )) ||
_geectl__subcmd__config__subcmd__import-oscrc_commands() {
    local commands; commands=()
    _describe -t commands 'geectl config import-oscrc commands' commands "$@"
}
(( $+functions[_geectl__subcmd__config__subcmd__init_commands] )) ||
_geectl__subcmd__config__subcmd__init_commands() {
    local commands; commands=()
    _describe -t commands 'geectl config init commands' commands "$@"
}
(( $+functions[_geectl__subcmd__config__subcmd__show_commands] )) ||
_geectl__subcmd__config__subcmd__show_commands() {
    local commands; commands=()
    _describe -t commands 'geectl config show commands' commands "$@"
}
(( $+functions[_geectl__subcmd__diff_commands] )) ||
_geectl__subcmd__diff_commands() {
    local commands; commands=()
    _describe -t commands 'geectl diff commands' commands "$@"
}
(( $+functions[_geectl__subcmd__gitea_commands] )) ||
_geectl__subcmd__gitea_commands() {
    local commands; commands=(
'repo-get:Show a repository\: owner, default branch, privacy, clone URLs, and the token owner'\''s permissions' \
'repo-list:List repositories\: the token owner'\''s own repos, one user'\''s repos with \`--owner\`, one organization'\''s with \`--org\`, or both (the user'\''s first, then the organization'\''s)' \
'repo-search:Search repositories across the instance by keyword\: the keyword matches the name and the description. The Gitea counterpart of \`remote search-packages\` / \`remote search-projects\`' \
'whoami:Show the token owner\: the login, full name, and email the configured Gitea token belongs to' \
'repo-create:Create a repository under the token owner'\''s account. A mutation\: dry-run is the default (only the plan is printed); \`--live\` performs the creation' \
'repo-delete:Delete a repository. A mutation\: dry-run is the default (only the plan is printed); \`--live\` performs the deletion. The library refuses without \`--force\`, even live' \
'repo-fork:Fork a repository into the token owner'\''s account (or an organization with \`--organization\`). A mutation\: dry-run is the default (only the plan is printed); \`--live\` performs the fork' \
'branch-list:List the branches of a repository' \
'branch-get:Show one branch\: name, head commit, protection' \
'branch-head:Print the head commit SHA of a branch. Content command\: the text IS the SHA, with no header' \
'raw-file:Print one file from a repository verbatim. Content command\: the text IS the file content, with no header. \`--ref\` reads from a branch, tag, or commit instead of the default branch. A missing file is \`NotFound\`, never empty content — the 404 is the existence answer' \
'commit-statuses:List a commit'\''s statuses\: the CI states reported for that commit' \
'maintainers:Show the maintainers recorded in a git project'\''s \`_maintainership.json\`\: the project maintainers and the per-package maintainers. The git-side counterpart of \`remote maintainers\`, read from the repo'\''s default branch' \
'cleanup-forks:List obsolete Gitea forks and their OBS home test projects, and delete them. A mutation\: dry-run is the default (only the plan is printed); \`--live\` performs the deletion. A project is deleted only when its description marker names a fork that no longer exists on Gitea; every other project is listed and left in place' \
'help:Print this message or the help of the given subcommand(s)' \
    )
    _describe -t commands 'geectl gitea commands' commands "$@"
}
(( $+functions[_geectl__subcmd__gitea__subcmd__branch-get_commands] )) ||
_geectl__subcmd__gitea__subcmd__branch-get_commands() {
    local commands; commands=()
    _describe -t commands 'geectl gitea branch-get commands' commands "$@"
}
(( $+functions[_geectl__subcmd__gitea__subcmd__branch-head_commands] )) ||
_geectl__subcmd__gitea__subcmd__branch-head_commands() {
    local commands; commands=()
    _describe -t commands 'geectl gitea branch-head commands' commands "$@"
}
(( $+functions[_geectl__subcmd__gitea__subcmd__branch-list_commands] )) ||
_geectl__subcmd__gitea__subcmd__branch-list_commands() {
    local commands; commands=()
    _describe -t commands 'geectl gitea branch-list commands' commands "$@"
}
(( $+functions[_geectl__subcmd__gitea__subcmd__cleanup-forks_commands] )) ||
_geectl__subcmd__gitea__subcmd__cleanup-forks_commands() {
    local commands; commands=()
    _describe -t commands 'geectl gitea cleanup-forks commands' commands "$@"
}
(( $+functions[_geectl__subcmd__gitea__subcmd__commit-statuses_commands] )) ||
_geectl__subcmd__gitea__subcmd__commit-statuses_commands() {
    local commands; commands=()
    _describe -t commands 'geectl gitea commit-statuses commands' commands "$@"
}
(( $+functions[_geectl__subcmd__gitea__subcmd__help_commands] )) ||
_geectl__subcmd__gitea__subcmd__help_commands() {
    local commands; commands=(
'repo-get:Show a repository\: owner, default branch, privacy, clone URLs, and the token owner'\''s permissions' \
'repo-list:List repositories\: the token owner'\''s own repos, one user'\''s repos with \`--owner\`, one organization'\''s with \`--org\`, or both (the user'\''s first, then the organization'\''s)' \
'repo-search:Search repositories across the instance by keyword\: the keyword matches the name and the description. The Gitea counterpart of \`remote search-packages\` / \`remote search-projects\`' \
'whoami:Show the token owner\: the login, full name, and email the configured Gitea token belongs to' \
'repo-create:Create a repository under the token owner'\''s account. A mutation\: dry-run is the default (only the plan is printed); \`--live\` performs the creation' \
'repo-delete:Delete a repository. A mutation\: dry-run is the default (only the plan is printed); \`--live\` performs the deletion. The library refuses without \`--force\`, even live' \
'repo-fork:Fork a repository into the token owner'\''s account (or an organization with \`--organization\`). A mutation\: dry-run is the default (only the plan is printed); \`--live\` performs the fork' \
'branch-list:List the branches of a repository' \
'branch-get:Show one branch\: name, head commit, protection' \
'branch-head:Print the head commit SHA of a branch. Content command\: the text IS the SHA, with no header' \
'raw-file:Print one file from a repository verbatim. Content command\: the text IS the file content, with no header. \`--ref\` reads from a branch, tag, or commit instead of the default branch. A missing file is \`NotFound\`, never empty content — the 404 is the existence answer' \
'commit-statuses:List a commit'\''s statuses\: the CI states reported for that commit' \
'maintainers:Show the maintainers recorded in a git project'\''s \`_maintainership.json\`\: the project maintainers and the per-package maintainers. The git-side counterpart of \`remote maintainers\`, read from the repo'\''s default branch' \
'cleanup-forks:List obsolete Gitea forks and their OBS home test projects, and delete them. A mutation\: dry-run is the default (only the plan is printed); \`--live\` performs the deletion. A project is deleted only when its description marker names a fork that no longer exists on Gitea; every other project is listed and left in place' \
'help:Print this message or the help of the given subcommand(s)' \
    )
    _describe -t commands 'geectl gitea help commands' commands "$@"
}
(( $+functions[_geectl__subcmd__gitea__subcmd__help__subcmd__branch-get_commands] )) ||
_geectl__subcmd__gitea__subcmd__help__subcmd__branch-get_commands() {
    local commands; commands=()
    _describe -t commands 'geectl gitea help branch-get commands' commands "$@"
}
(( $+functions[_geectl__subcmd__gitea__subcmd__help__subcmd__branch-head_commands] )) ||
_geectl__subcmd__gitea__subcmd__help__subcmd__branch-head_commands() {
    local commands; commands=()
    _describe -t commands 'geectl gitea help branch-head commands' commands "$@"
}
(( $+functions[_geectl__subcmd__gitea__subcmd__help__subcmd__branch-list_commands] )) ||
_geectl__subcmd__gitea__subcmd__help__subcmd__branch-list_commands() {
    local commands; commands=()
    _describe -t commands 'geectl gitea help branch-list commands' commands "$@"
}
(( $+functions[_geectl__subcmd__gitea__subcmd__help__subcmd__cleanup-forks_commands] )) ||
_geectl__subcmd__gitea__subcmd__help__subcmd__cleanup-forks_commands() {
    local commands; commands=()
    _describe -t commands 'geectl gitea help cleanup-forks commands' commands "$@"
}
(( $+functions[_geectl__subcmd__gitea__subcmd__help__subcmd__commit-statuses_commands] )) ||
_geectl__subcmd__gitea__subcmd__help__subcmd__commit-statuses_commands() {
    local commands; commands=()
    _describe -t commands 'geectl gitea help commit-statuses commands' commands "$@"
}
(( $+functions[_geectl__subcmd__gitea__subcmd__help__subcmd__help_commands] )) ||
_geectl__subcmd__gitea__subcmd__help__subcmd__help_commands() {
    local commands; commands=()
    _describe -t commands 'geectl gitea help help commands' commands "$@"
}
(( $+functions[_geectl__subcmd__gitea__subcmd__help__subcmd__maintainers_commands] )) ||
_geectl__subcmd__gitea__subcmd__help__subcmd__maintainers_commands() {
    local commands; commands=()
    _describe -t commands 'geectl gitea help maintainers commands' commands "$@"
}
(( $+functions[_geectl__subcmd__gitea__subcmd__help__subcmd__raw-file_commands] )) ||
_geectl__subcmd__gitea__subcmd__help__subcmd__raw-file_commands() {
    local commands; commands=()
    _describe -t commands 'geectl gitea help raw-file commands' commands "$@"
}
(( $+functions[_geectl__subcmd__gitea__subcmd__help__subcmd__repo-create_commands] )) ||
_geectl__subcmd__gitea__subcmd__help__subcmd__repo-create_commands() {
    local commands; commands=()
    _describe -t commands 'geectl gitea help repo-create commands' commands "$@"
}
(( $+functions[_geectl__subcmd__gitea__subcmd__help__subcmd__repo-delete_commands] )) ||
_geectl__subcmd__gitea__subcmd__help__subcmd__repo-delete_commands() {
    local commands; commands=()
    _describe -t commands 'geectl gitea help repo-delete commands' commands "$@"
}
(( $+functions[_geectl__subcmd__gitea__subcmd__help__subcmd__repo-fork_commands] )) ||
_geectl__subcmd__gitea__subcmd__help__subcmd__repo-fork_commands() {
    local commands; commands=()
    _describe -t commands 'geectl gitea help repo-fork commands' commands "$@"
}
(( $+functions[_geectl__subcmd__gitea__subcmd__help__subcmd__repo-get_commands] )) ||
_geectl__subcmd__gitea__subcmd__help__subcmd__repo-get_commands() {
    local commands; commands=()
    _describe -t commands 'geectl gitea help repo-get commands' commands "$@"
}
(( $+functions[_geectl__subcmd__gitea__subcmd__help__subcmd__repo-list_commands] )) ||
_geectl__subcmd__gitea__subcmd__help__subcmd__repo-list_commands() {
    local commands; commands=()
    _describe -t commands 'geectl gitea help repo-list commands' commands "$@"
}
(( $+functions[_geectl__subcmd__gitea__subcmd__help__subcmd__repo-search_commands] )) ||
_geectl__subcmd__gitea__subcmd__help__subcmd__repo-search_commands() {
    local commands; commands=()
    _describe -t commands 'geectl gitea help repo-search commands' commands "$@"
}
(( $+functions[_geectl__subcmd__gitea__subcmd__help__subcmd__whoami_commands] )) ||
_geectl__subcmd__gitea__subcmd__help__subcmd__whoami_commands() {
    local commands; commands=()
    _describe -t commands 'geectl gitea help whoami commands' commands "$@"
}
(( $+functions[_geectl__subcmd__gitea__subcmd__maintainers_commands] )) ||
_geectl__subcmd__gitea__subcmd__maintainers_commands() {
    local commands; commands=()
    _describe -t commands 'geectl gitea maintainers commands' commands "$@"
}
(( $+functions[_geectl__subcmd__gitea__subcmd__raw-file_commands] )) ||
_geectl__subcmd__gitea__subcmd__raw-file_commands() {
    local commands; commands=()
    _describe -t commands 'geectl gitea raw-file commands' commands "$@"
}
(( $+functions[_geectl__subcmd__gitea__subcmd__repo-create_commands] )) ||
_geectl__subcmd__gitea__subcmd__repo-create_commands() {
    local commands; commands=()
    _describe -t commands 'geectl gitea repo-create commands' commands "$@"
}
(( $+functions[_geectl__subcmd__gitea__subcmd__repo-delete_commands] )) ||
_geectl__subcmd__gitea__subcmd__repo-delete_commands() {
    local commands; commands=()
    _describe -t commands 'geectl gitea repo-delete commands' commands "$@"
}
(( $+functions[_geectl__subcmd__gitea__subcmd__repo-fork_commands] )) ||
_geectl__subcmd__gitea__subcmd__repo-fork_commands() {
    local commands; commands=()
    _describe -t commands 'geectl gitea repo-fork commands' commands "$@"
}
(( $+functions[_geectl__subcmd__gitea__subcmd__repo-get_commands] )) ||
_geectl__subcmd__gitea__subcmd__repo-get_commands() {
    local commands; commands=()
    _describe -t commands 'geectl gitea repo-get commands' commands "$@"
}
(( $+functions[_geectl__subcmd__gitea__subcmd__repo-list_commands] )) ||
_geectl__subcmd__gitea__subcmd__repo-list_commands() {
    local commands; commands=()
    _describe -t commands 'geectl gitea repo-list commands' commands "$@"
}
(( $+functions[_geectl__subcmd__gitea__subcmd__repo-search_commands] )) ||
_geectl__subcmd__gitea__subcmd__repo-search_commands() {
    local commands; commands=()
    _describe -t commands 'geectl gitea repo-search commands' commands "$@"
}
(( $+functions[_geectl__subcmd__gitea__subcmd__whoami_commands] )) ||
_geectl__subcmd__gitea__subcmd__whoami_commands() {
    local commands; commands=()
    _describe -t commands 'geectl gitea whoami commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help_commands] )) ||
_geectl__subcmd__help_commands() {
    local commands; commands=(
'version:Print version information as the result payload' \
'config:Inspect or initialise the config file' \
'remote:Remote reads and mutations\: inspect what the distribution holds, and change it' \
'request:Read and act on OBS submit requests, and file new ones' \
'branch:Branch, copy, and link packages' \
'build:Build-status commands\: results, logs, histories, binaries, buildconfig' \
'local-build:Local builds in the podman build vertical\: build a package, run its \`_service\` services, the precommit gate, \`spec-cleaner\`, wipe a build root, prune the cache, manage the daily periodic cleanup, enter the preserved build root, and read the log of the last local build' \
'maintenance:Maintenance operations\: maintained-instances listing and maintenance branching' \
'service:Service reads and trigger\: list a package'\''s services, their run state, and trigger a server-side run' \
'gitea:Gitea operations\: repository and branch reads and mutations on the configured instance' \
'pull-request:Read and act on pull requests\: OBS submit requests and Gitea pull requests as one surface' \
'person:Search for a person\: the OBS projects naming the login and the Gitea users matching the keyword, as one operation' \
'meta:Set the meta of an OBS project, an OBS package, or a Gitea repository as one operation\: the target selects the backend' \
'changelog:Validate and generate \`.changes\` entries in a package directory, pre-file' \
'checkout:Check out a package into a git working copy, under the current directory' \
'update:Update the working copy in the current directory to the latest base' \
'status:Show the working copy'\''s state against the base' \
'diff:Show the diff of the worktree and the index against the base, plus the link diff on a linked checkout' \
'add:Stage files for commit' \
'rm:Remove files from the working copy' \
'addremove:Stage new files and worktree deletions' \
'commit:Commit the staged changes' \
'pull:Pull a linked package to the link target'\''s latest revision' \
'resolve:Stage the resolved files and continue a stopped rebase' \
'prjconf:Read, validate and clean a project'\''s prjconf' \
'help:Print this message or the help of the given subcommand(s)' \
    )
    _describe -t commands 'geectl help commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__add_commands] )) ||
_geectl__subcmd__help__subcmd__add_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help add commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__addremove_commands] )) ||
_geectl__subcmd__help__subcmd__addremove_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help addremove commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__branch_commands] )) ||
_geectl__subcmd__help__subcmd__branch_commands() {
    local commands; commands=(
'create:Branch a git-backed package\: Gitea fork + OBS fork into a test project with the source project'\''s repositories. A mutation\: dry-run is the default (only the plan is printed); \`--live\` performs the branch' \
'branch-package:Branch a package\: create a linked copy under a branch project. A mutation\: dry-run is the default (only the plan is printed); \`--live\` performs the branch' \
'copy-package:Copy a package to another project (or another OBS instance with \`--to-apiurl\`). A mutation\: dry-run is the default' \
'copy-project:Copy a whole project. A mutation\: dry-run is the default' \
'set-devel-project:Set (or, without \`--devel-project\`, remove) the devel project of a package. A mutation\: dry-run is the default' \
'link-package:Link a package to another package. A mutation\: dry-run is the default' \
'repair-link:Repair a broken link\: strip the failing patches from \`_link\` server-side and pin it at a revision where it expands. A mutation\: dry-run is the default' \
'set-link-rev:Set the revision a link expands at. A mutation\: dry-run is the default' \
'set-link-rev-project:Set the revision every linked package in a project expands at. A mutation\: dry-run is the default' \
'link-to-branch:Turn a link into a branch\: replace the link with a copy of the linked files. A mutation\: dry-run is the default' \
'detach-branch:Detach a branch\: replace the branch link with a copy of the branched files. A mutation\: dry-run is the default' \
    )
    _describe -t commands 'geectl help branch commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__branch__subcmd__branch-package_commands] )) ||
_geectl__subcmd__help__subcmd__branch__subcmd__branch-package_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help branch branch-package commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__branch__subcmd__copy-package_commands] )) ||
_geectl__subcmd__help__subcmd__branch__subcmd__copy-package_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help branch copy-package commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__branch__subcmd__copy-project_commands] )) ||
_geectl__subcmd__help__subcmd__branch__subcmd__copy-project_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help branch copy-project commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__branch__subcmd__create_commands] )) ||
_geectl__subcmd__help__subcmd__branch__subcmd__create_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help branch create commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__branch__subcmd__detach-branch_commands] )) ||
_geectl__subcmd__help__subcmd__branch__subcmd__detach-branch_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help branch detach-branch commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__branch__subcmd__link-package_commands] )) ||
_geectl__subcmd__help__subcmd__branch__subcmd__link-package_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help branch link-package commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__branch__subcmd__link-to-branch_commands] )) ||
_geectl__subcmd__help__subcmd__branch__subcmd__link-to-branch_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help branch link-to-branch commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__branch__subcmd__repair-link_commands] )) ||
_geectl__subcmd__help__subcmd__branch__subcmd__repair-link_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help branch repair-link commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__branch__subcmd__set-devel-project_commands] )) ||
_geectl__subcmd__help__subcmd__branch__subcmd__set-devel-project_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help branch set-devel-project commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__branch__subcmd__set-link-rev_commands] )) ||
_geectl__subcmd__help__subcmd__branch__subcmd__set-link-rev_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help branch set-link-rev commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__branch__subcmd__set-link-rev-project_commands] )) ||
_geectl__subcmd__help__subcmd__branch__subcmd__set-link-rev-project_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help branch set-link-rev-project commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__build_commands] )) ||
_geectl__subcmd__help__subcmd__build_commands() {
    local commands; commands=(
'results:Read the build-results matrix for a project' \
'wait-for-results:Poll the build results until they settle, or the deadline passes' \
'build-log:Read the build log for a package' \
'build-status:Read the build status code for a package' \
'job-status:Read the running build job for a package, if any' \
'rebuild-package:Rebuild a package\: rerun a build without touching the sources. A mutation\: dry-run is the default (only the plan is printed); \`--live\` performs the rebuild' \
'restart-build:Restart the running build of a package, without touching the sources. A mutation\: dry-run is the default (only the plan is printed); \`--live\` performs the restart' \
'abort-build:Abort the running build of a package. A mutation\: dry-run is the default (only the plan is printed); \`--live\` performs the abort' \
'wipe-binaries:Wipe the binary packages of a project or a package. A mutation\: dry-run is the default (only the plan is printed); \`--live\` performs the wipe. At least one scope selector is required\: \`--all\`, \`--arch\`, \`--repository\`, or a status filter' \
'buildinfo:Read the buildinfo for a package\: the blocking error, if any, and the dependencies the build pulls in' \
'buildconfig:Read the build configuration for a project/repository\: the expanded \`_buildconfig\` text the engine takes as \`--dist\`' \
'build-history:List the completed builds for a package, newest first' \
'job-history:List the build jobs that ran in a repository, newest first' \
'list-binaries:List the built files for a package (or the whole repository)' \
'download-binary:Download one built file by name. With \`--all\`, every built file' \
'fileinfo:Read the metadata of a built file\: RPM header fields plus dependency lists' \
'dependson:List which packages each package depends on for its build' \
'whatdependson:List the packages that depend on a package for their build' \
'check-constraints:List the worker ids satisfying a \`_constraints\` document' \
'list-repositories:List the repositories of a project' \
    )
    _describe -t commands 'geectl help build commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__build__subcmd__abort-build_commands] )) ||
_geectl__subcmd__help__subcmd__build__subcmd__abort-build_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help build abort-build commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__build__subcmd__build-history_commands] )) ||
_geectl__subcmd__help__subcmd__build__subcmd__build-history_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help build build-history commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__build__subcmd__build-log_commands] )) ||
_geectl__subcmd__help__subcmd__build__subcmd__build-log_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help build build-log commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__build__subcmd__build-status_commands] )) ||
_geectl__subcmd__help__subcmd__build__subcmd__build-status_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help build build-status commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__build__subcmd__buildconfig_commands] )) ||
_geectl__subcmd__help__subcmd__build__subcmd__buildconfig_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help build buildconfig commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__build__subcmd__buildinfo_commands] )) ||
_geectl__subcmd__help__subcmd__build__subcmd__buildinfo_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help build buildinfo commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__build__subcmd__check-constraints_commands] )) ||
_geectl__subcmd__help__subcmd__build__subcmd__check-constraints_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help build check-constraints commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__build__subcmd__dependson_commands] )) ||
_geectl__subcmd__help__subcmd__build__subcmd__dependson_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help build dependson commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__build__subcmd__download-binary_commands] )) ||
_geectl__subcmd__help__subcmd__build__subcmd__download-binary_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help build download-binary commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__build__subcmd__fileinfo_commands] )) ||
_geectl__subcmd__help__subcmd__build__subcmd__fileinfo_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help build fileinfo commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__build__subcmd__job-history_commands] )) ||
_geectl__subcmd__help__subcmd__build__subcmd__job-history_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help build job-history commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__build__subcmd__job-status_commands] )) ||
_geectl__subcmd__help__subcmd__build__subcmd__job-status_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help build job-status commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__build__subcmd__list-binaries_commands] )) ||
_geectl__subcmd__help__subcmd__build__subcmd__list-binaries_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help build list-binaries commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__build__subcmd__list-repositories_commands] )) ||
_geectl__subcmd__help__subcmd__build__subcmd__list-repositories_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help build list-repositories commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__build__subcmd__rebuild-package_commands] )) ||
_geectl__subcmd__help__subcmd__build__subcmd__rebuild-package_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help build rebuild-package commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__build__subcmd__restart-build_commands] )) ||
_geectl__subcmd__help__subcmd__build__subcmd__restart-build_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help build restart-build commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__build__subcmd__results_commands] )) ||
_geectl__subcmd__help__subcmd__build__subcmd__results_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help build results commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__build__subcmd__wait-for-results_commands] )) ||
_geectl__subcmd__help__subcmd__build__subcmd__wait-for-results_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help build wait-for-results commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__build__subcmd__whatdependson_commands] )) ||
_geectl__subcmd__help__subcmd__build__subcmd__whatdependson_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help build whatdependson commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__build__subcmd__wipe-binaries_commands] )) ||
_geectl__subcmd__help__subcmd__build__subcmd__wipe-binaries_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help build wipe-binaries commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__changelog_commands] )) ||
_geectl__subcmd__help__subcmd__changelog_commands() {
    local commands; commands=(
'validate:Validate the \`.changes\` entries in a package directory against the V1–V12 rules. Local-only by default; with \`--target-project\`/\`--target-package\` the target-relative rules (V2, V4, V5 newest-vs-target, V11 older entries) run against the target'\''s current files. Exits non-zero naming the failing rule' \
'generate:Generate a skeleton \`.changes\` entry from the diff against the target (version line, patch and dependency lists, license lines) plus the upstream NEWS section for the new version, extracted from the source tarball. The output is a draft\: what the diff cannot supply arrives as explicit \`TODO\` slots for a human to fill. \`--write\` prepends the draft into the package'\''s \`.changes\` file, and refuses an incomplete draft (V9 needs-detail)' \
'identity:Print the resolved \`.changes\` entry author in \`Name <email>\` form — the same value a bare \`changelog generate\` would write. The author resolves through the identity chain (#405)\: config \`\[identity\]\`, then \`VC_REALNAME\` / \`VC_MAILADDR\`, then the OBS account, then git identity; the command refuses when none yields an identity' \
    )
    _describe -t commands 'geectl help changelog commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__changelog__subcmd__generate_commands] )) ||
_geectl__subcmd__help__subcmd__changelog__subcmd__generate_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help changelog generate commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__changelog__subcmd__identity_commands] )) ||
_geectl__subcmd__help__subcmd__changelog__subcmd__identity_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help changelog identity commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__changelog__subcmd__validate_commands] )) ||
_geectl__subcmd__help__subcmd__changelog__subcmd__validate_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help changelog validate commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__checkout_commands] )) ||
_geectl__subcmd__help__subcmd__checkout_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help checkout commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__commit_commands] )) ||
_geectl__subcmd__help__subcmd__commit_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help commit commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__config_commands] )) ||
_geectl__subcmd__help__subcmd__config_commands() {
    local commands; commands=(
'show:Show the effective config\: endpoint URLs with their source, and credential status (never a secret value)' \
'init:Write the config template (endpoints only). Refuses if the file already exists' \
'import-oscrc:Import one instance mapping from an oscrc file (one-time migration). Reads the oscrc file once, on explicit request — geectl never reads it at runtime — and writes only the alias to apiurl mapping into the config \`\[instances\]\` table. The password is never stored and never printed\: set it with the printed \`GEECTL_OBS_PASSWORD_<ALIAS>\` variable yourself. The username is not written into the config either\: it stays in the environment as \`GEECTL_OBS_USERNAME_<ALIAS>\`. Dry-run is the default (only the plan is printed); \`--live\` writes the config' \
    )
    _describe -t commands 'geectl help config commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__config__subcmd__import-oscrc_commands] )) ||
_geectl__subcmd__help__subcmd__config__subcmd__import-oscrc_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help config import-oscrc commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__config__subcmd__init_commands] )) ||
_geectl__subcmd__help__subcmd__config__subcmd__init_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help config init commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__config__subcmd__show_commands] )) ||
_geectl__subcmd__help__subcmd__config__subcmd__show_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help config show commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__diff_commands] )) ||
_geectl__subcmd__help__subcmd__diff_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help diff commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__gitea_commands] )) ||
_geectl__subcmd__help__subcmd__gitea_commands() {
    local commands; commands=(
'repo-get:Show a repository\: owner, default branch, privacy, clone URLs, and the token owner'\''s permissions' \
'repo-list:List repositories\: the token owner'\''s own repos, one user'\''s repos with \`--owner\`, one organization'\''s with \`--org\`, or both (the user'\''s first, then the organization'\''s)' \
'repo-search:Search repositories across the instance by keyword\: the keyword matches the name and the description. The Gitea counterpart of \`remote search-packages\` / \`remote search-projects\`' \
'whoami:Show the token owner\: the login, full name, and email the configured Gitea token belongs to' \
'repo-create:Create a repository under the token owner'\''s account. A mutation\: dry-run is the default (only the plan is printed); \`--live\` performs the creation' \
'repo-delete:Delete a repository. A mutation\: dry-run is the default (only the plan is printed); \`--live\` performs the deletion. The library refuses without \`--force\`, even live' \
'repo-fork:Fork a repository into the token owner'\''s account (or an organization with \`--organization\`). A mutation\: dry-run is the default (only the plan is printed); \`--live\` performs the fork' \
'branch-list:List the branches of a repository' \
'branch-get:Show one branch\: name, head commit, protection' \
'branch-head:Print the head commit SHA of a branch. Content command\: the text IS the SHA, with no header' \
'raw-file:Print one file from a repository verbatim. Content command\: the text IS the file content, with no header. \`--ref\` reads from a branch, tag, or commit instead of the default branch. A missing file is \`NotFound\`, never empty content — the 404 is the existence answer' \
'commit-statuses:List a commit'\''s statuses\: the CI states reported for that commit' \
'maintainers:Show the maintainers recorded in a git project'\''s \`_maintainership.json\`\: the project maintainers and the per-package maintainers. The git-side counterpart of \`remote maintainers\`, read from the repo'\''s default branch' \
'cleanup-forks:List obsolete Gitea forks and their OBS home test projects, and delete them. A mutation\: dry-run is the default (only the plan is printed); \`--live\` performs the deletion. A project is deleted only when its description marker names a fork that no longer exists on Gitea; every other project is listed and left in place' \
    )
    _describe -t commands 'geectl help gitea commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__gitea__subcmd__branch-get_commands] )) ||
_geectl__subcmd__help__subcmd__gitea__subcmd__branch-get_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help gitea branch-get commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__gitea__subcmd__branch-head_commands] )) ||
_geectl__subcmd__help__subcmd__gitea__subcmd__branch-head_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help gitea branch-head commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__gitea__subcmd__branch-list_commands] )) ||
_geectl__subcmd__help__subcmd__gitea__subcmd__branch-list_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help gitea branch-list commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__gitea__subcmd__cleanup-forks_commands] )) ||
_geectl__subcmd__help__subcmd__gitea__subcmd__cleanup-forks_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help gitea cleanup-forks commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__gitea__subcmd__commit-statuses_commands] )) ||
_geectl__subcmd__help__subcmd__gitea__subcmd__commit-statuses_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help gitea commit-statuses commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__gitea__subcmd__maintainers_commands] )) ||
_geectl__subcmd__help__subcmd__gitea__subcmd__maintainers_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help gitea maintainers commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__gitea__subcmd__raw-file_commands] )) ||
_geectl__subcmd__help__subcmd__gitea__subcmd__raw-file_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help gitea raw-file commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__gitea__subcmd__repo-create_commands] )) ||
_geectl__subcmd__help__subcmd__gitea__subcmd__repo-create_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help gitea repo-create commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__gitea__subcmd__repo-delete_commands] )) ||
_geectl__subcmd__help__subcmd__gitea__subcmd__repo-delete_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help gitea repo-delete commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__gitea__subcmd__repo-fork_commands] )) ||
_geectl__subcmd__help__subcmd__gitea__subcmd__repo-fork_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help gitea repo-fork commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__gitea__subcmd__repo-get_commands] )) ||
_geectl__subcmd__help__subcmd__gitea__subcmd__repo-get_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help gitea repo-get commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__gitea__subcmd__repo-list_commands] )) ||
_geectl__subcmd__help__subcmd__gitea__subcmd__repo-list_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help gitea repo-list commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__gitea__subcmd__repo-search_commands] )) ||
_geectl__subcmd__help__subcmd__gitea__subcmd__repo-search_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help gitea repo-search commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__gitea__subcmd__whoami_commands] )) ||
_geectl__subcmd__help__subcmd__gitea__subcmd__whoami_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help gitea whoami commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__help_commands] )) ||
_geectl__subcmd__help__subcmd__help_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help help commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__local-build_commands] )) ||
_geectl__subcmd__help__subcmd__local-build_commands() {
    local commands; commands=(
'service-run:Run the \`_service\` file'\''s services in the container, filtered by the run mode' \
'precommit:The precommit gate\: the \`precommit\`-mode service run, then \`pre_checkin.sh\` when the source directory carries one' \
'spec-cleaner:Run \`spec-cleaner\` on a spec in the container' \
'build:Build the package in the container\: the run-mode service run, the precommit gate, then the engine build' \
'wipe:Wipe a build root\: delete its contents (the lock file stays). A mutation\: dry-run is the default (only the plan is printed); \`--live\` performs the wipe. The marker check and the in-use lock are safety invariants, never bypassed' \
'prune:Print the eviction plan (what would be evicted, in order, with bytes); delete only with \`--apply\`. The plan is the default\: the leaf'\''s own \`--apply\` is the confirmation, so the scheduled task runs exactly \`prune --apply\`' \
'periodic-install:Install the daily periodic cleanup for this platform\: write the scheduler entry and enable it. A mutation\: dry-run is the default (only the plan is printed); \`--live\` performs the install' \
'periodic-uninstall:Remove the daily periodic cleanup for this platform\: disable the scheduler entry, then remove its files. A mutation\: dry-run is the default (only the plan is printed); \`--live\` performs the uninstall' \
'periodic-status:Show whether the daily periodic cleanup is installed and enabled on this platform. Read-only' \
'chroot:Enter the preserved build root of the last build for the package and run repro commands there, read from stdin, one per line, joined with \`&&\` into one shell command. A mutation\: dry-run is the default (only the plan is printed); \`--live\` runs the commands in the root. A terminal on stdin is refused instead of blocking on a read that never arrives. A failing repro still exits 0\: the exit code is data, in \`exit\:\` and in \`record.result.exit_code\`' \
'log:Read the log of the last local build\: the \`<root>/.build.log\` the build'\''s coordinates name. A read\: no \`--live\` needed. The text is the log verbatim (design rule 4); the verdict, exit code, log path, and truncation flag ride in the JSON payload' \
    )
    _describe -t commands 'geectl help local-build commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__local-build__subcmd__build_commands] )) ||
_geectl__subcmd__help__subcmd__local-build__subcmd__build_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help local-build build commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__local-build__subcmd__chroot_commands] )) ||
_geectl__subcmd__help__subcmd__local-build__subcmd__chroot_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help local-build chroot commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__local-build__subcmd__log_commands] )) ||
_geectl__subcmd__help__subcmd__local-build__subcmd__log_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help local-build log commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__local-build__subcmd__periodic-install_commands] )) ||
_geectl__subcmd__help__subcmd__local-build__subcmd__periodic-install_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help local-build periodic-install commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__local-build__subcmd__periodic-status_commands] )) ||
_geectl__subcmd__help__subcmd__local-build__subcmd__periodic-status_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help local-build periodic-status commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__local-build__subcmd__periodic-uninstall_commands] )) ||
_geectl__subcmd__help__subcmd__local-build__subcmd__periodic-uninstall_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help local-build periodic-uninstall commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__local-build__subcmd__precommit_commands] )) ||
_geectl__subcmd__help__subcmd__local-build__subcmd__precommit_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help local-build precommit commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__local-build__subcmd__prune_commands] )) ||
_geectl__subcmd__help__subcmd__local-build__subcmd__prune_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help local-build prune commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__local-build__subcmd__service-run_commands] )) ||
_geectl__subcmd__help__subcmd__local-build__subcmd__service-run_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help local-build service-run commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__local-build__subcmd__spec-cleaner_commands] )) ||
_geectl__subcmd__help__subcmd__local-build__subcmd__spec-cleaner_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help local-build spec-cleaner commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__local-build__subcmd__wipe_commands] )) ||
_geectl__subcmd__help__subcmd__local-build__subcmd__wipe_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help local-build wipe commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__maintenance_commands] )) ||
_geectl__subcmd__help__subcmd__maintenance_commands() {
    local commands; commands=(
'maintained-instances:List the maintained instances of a package\: the projects carrying the OBS\:Maintained attribute that the package exists in, with where a branch would land. Read-only; requires OBS credentials' \
'maintenance-branch:Branch a maintained package into its maintenance branch project and report where it landed. A mutation\: dry-run is the default (only the plan is printed); the global \`--live\` performs the branch' \
    )
    _describe -t commands 'geectl help maintenance commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__maintenance__subcmd__maintained-instances_commands] )) ||
_geectl__subcmd__help__subcmd__maintenance__subcmd__maintained-instances_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help maintenance maintained-instances commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__maintenance__subcmd__maintenance-branch_commands] )) ||
_geectl__subcmd__help__subcmd__maintenance__subcmd__maintenance-branch_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help maintenance maintenance-branch commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__meta_commands] )) ||
_geectl__subcmd__help__subcmd__meta_commands() {
    local commands; commands=(
'set:Set the meta of an OBS project, an OBS package, or a Gitea repository\: the description and/or URL. The target selects the backend\: \`--project\` (with \`--package\` for a package) writes OBS meta, \`--repo\` writes Gitea repo settings. A mutation\: dry-run is the default (only the plan is printed); \`--live\` performs the write. Only the given fields change; the rest of the meta is kept as it is. There is no flag to clear a field' \
    )
    _describe -t commands 'geectl help meta commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__meta__subcmd__set_commands] )) ||
_geectl__subcmd__help__subcmd__meta__subcmd__set_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help meta set commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__person_commands] )) ||
_geectl__subcmd__help__subcmd__person_commands() {
    local commands; commands=(
'search:Search for a person\: the OBS projects naming the login, and the Gitea users matching the keyword. With no \`--backend\` both arms run, OBS first then Gitea' \
    )
    _describe -t commands 'geectl help person commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__person__subcmd__search_commands] )) ||
_geectl__subcmd__help__subcmd__person__subcmd__search_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help person search commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__prjconf_commands] )) ||
_geectl__subcmd__help__subcmd__prjconf_commands() {
    local commands; commands=(
'show:Show a project'\''s raw prjconf' \
'validate:Validate a project'\''s prjconf across its repositories. Exits 24 when error findings exist' \
'clean:Clean a project'\''s prjconf. A mutation\: dry-run is the default. Shares validate'\''s credential gate and 32-repository bound' \
    )
    _describe -t commands 'geectl help prjconf commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__prjconf__subcmd__clean_commands] )) ||
_geectl__subcmd__help__subcmd__prjconf__subcmd__clean_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help prjconf clean commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__prjconf__subcmd__show_commands] )) ||
_geectl__subcmd__help__subcmd__prjconf__subcmd__show_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help prjconf show commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__prjconf__subcmd__validate_commands] )) ||
_geectl__subcmd__help__subcmd__prjconf__subcmd__validate_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help prjconf validate commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__pull_commands] )) ||
_geectl__subcmd__help__subcmd__pull_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help pull commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__pull-request_commands] )) ||
_geectl__subcmd__help__subcmd__pull-request_commands() {
    local commands; commands=(
'show:Show one pull request\: \`obs\:<id>\` or \`gitea\:<owner>/<repo>#<number>\`' \
'diff:Show the diff of a pull request\: the unified diff, like \`request diff\`. With \`--xml\` the OBS arm renders the request XML with the diff injected; \`--xml\` is refused on a Gitea pull request, which has no request XML. A read, always live' \
'search:Search pull requests\: OBS submit requests and Gitea pull requests as one list, OBS first then Gitea. With no \`--backend\` both arms run; \`--backend\` scopes to one' \
'approve:Approve a pull request as the review asked of the target\: with no target flag, as the review asked of the acting user (OBS) or the token owner (Gitea). Dry-run by default; mutates only with \`--live\`' \
'request-changes:Ask for changes on a pull request, as \`approve\` but with the review declined instead of accepted. Dry-run by default; mutates only with \`--live\`' \
'comment:Comment on a pull request. Dry-run by default; mutates only with \`--live\`' \
'build-comment:Post the build-link comment on a Gitea pull request\: the OBS build results for the test project and package, as a link plus a one-line per-instance summary. The comment is posted once, on PR opening; there is no polling, no live update, and no edit. Dry-run by default; mutates only with \`--live\`' \
'comments:List the comments on a pull request, oldest first. A read, always live' \
'cancel-review-request:Cancel pending review requests on a pull request. Dry-run by default; mutates only with \`--live\`. On OBS the cancel fails closed' \
'merge:Merge a pull request. On OBS this accepts the submit request and forwards it to its link parent; a refused forward is reported and the accepted request stands. On Gitea the forward is resolved and validated before the merge lands, and the merged pull request is forwarded to its link parent too, unless \`--no-forward\` is given. The forwarded request'\''s \`.changes\` entry is the original description on the OBS arm and the pull request'\''s text on the Gitea arm, overridable with \`-m\` / \`--forward-message\`. Dry-run by default; mutates only with \`--live\`' \
'decline:Decline a pull request. On OBS this changes the request state to declined; on Gitea it closes the pull request. Dry-run by default; mutates only with \`--live\`' \
'withdraw:Withdraw a pull request. On OBS this revokes the request; on Gitea it closes the pull request. Dry-run by default; mutates only with \`--live\`' \
'reopen:Reopen a pull request. On OBS this changes a declined request back to \`new\`; on Gitea it reopens the closed pull request. Dry-run by default; mutates only with \`--live\`' \
'submit:File a submission from the working copy in the current directory\: an OBS submit request from an OBS-native checkout, a Gitea pull request from a git-backed one. The checkout'\''s backend decides; the other arm'\''s flags are refused. Dry-run by default; mutates only with \`--live\`' \
'cleanup:Clean the local build roots for the submit request filed from the working copy in the current directory, when that request is accepted. The request id was recorded by \`submit\` in the checkout; the leaf reads the request state once and wipes the roots only on \`accepted\`. A request in any other state, and a checkout with nothing recorded, delete nothing. Dry-run by default; mutates only with \`--live\`' \
    )
    _describe -t commands 'geectl help pull-request commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__pull-request__subcmd__approve_commands] )) ||
_geectl__subcmd__help__subcmd__pull-request__subcmd__approve_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help pull-request approve commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__pull-request__subcmd__build-comment_commands] )) ||
_geectl__subcmd__help__subcmd__pull-request__subcmd__build-comment_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help pull-request build-comment commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__pull-request__subcmd__cancel-review-request_commands] )) ||
_geectl__subcmd__help__subcmd__pull-request__subcmd__cancel-review-request_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help pull-request cancel-review-request commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__pull-request__subcmd__cleanup_commands] )) ||
_geectl__subcmd__help__subcmd__pull-request__subcmd__cleanup_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help pull-request cleanup commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__pull-request__subcmd__comment_commands] )) ||
_geectl__subcmd__help__subcmd__pull-request__subcmd__comment_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help pull-request comment commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__pull-request__subcmd__comments_commands] )) ||
_geectl__subcmd__help__subcmd__pull-request__subcmd__comments_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help pull-request comments commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__pull-request__subcmd__decline_commands] )) ||
_geectl__subcmd__help__subcmd__pull-request__subcmd__decline_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help pull-request decline commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__pull-request__subcmd__diff_commands] )) ||
_geectl__subcmd__help__subcmd__pull-request__subcmd__diff_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help pull-request diff commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__pull-request__subcmd__merge_commands] )) ||
_geectl__subcmd__help__subcmd__pull-request__subcmd__merge_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help pull-request merge commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__pull-request__subcmd__reopen_commands] )) ||
_geectl__subcmd__help__subcmd__pull-request__subcmd__reopen_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help pull-request reopen commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__pull-request__subcmd__request-changes_commands] )) ||
_geectl__subcmd__help__subcmd__pull-request__subcmd__request-changes_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help pull-request request-changes commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__pull-request__subcmd__search_commands] )) ||
_geectl__subcmd__help__subcmd__pull-request__subcmd__search_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help pull-request search commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__pull-request__subcmd__show_commands] )) ||
_geectl__subcmd__help__subcmd__pull-request__subcmd__show_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help pull-request show commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__pull-request__subcmd__submit_commands] )) ||
_geectl__subcmd__help__subcmd__pull-request__subcmd__submit_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help pull-request submit commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__pull-request__subcmd__withdraw_commands] )) ||
_geectl__subcmd__help__subcmd__pull-request__subcmd__withdraw_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help pull-request withdraw commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__remote_commands] )) ||
_geectl__subcmd__help__subcmd__remote_commands() {
    local commands; commands=(
'cat:Print a source file to standard output. Text only\: the library reads the file as a UTF-8 string, so a binary file fails the call with a server error ("response body is not valid UTF-8") instead of printing bytes' \
'download:Download a source file to a local path. The binary counterpart of \`cat\`\: the library streams the file to disk instead of reading it as a UTF-8 string, so a binary file (a source tarball) downloads instead of failing the UTF-8 check, and a file larger than the transport'\''s 10 MiB body cap downloads instead of failing. A read\: \`--live\` is ignored, like \`cat\`' \
'list-files:List the files of a package' \
'list-packages:List the packages of a project' \
'list-projects:List the top-level projects on the instance' \
'list-staging-projects:List the staging projects of a workflow project' \
'staging-project:Show one staging project of a workflow project' \
'package-log:Show the commit history of a package' \
'revision-patch:Show the per-file diff of one revision against the previous one. Computed client-side from per-file reads (the library lists both revisions and cats each changed file); a differing binary file fails the call' \
'project-meta:Show a project'\''s meta (title, description, persons, repositories)' \
'set-project-meta:Set a project'\''s meta, field by field or whole from a file. A mutation\: dry-run is the default (only the plan is printed); \`--live\` performs the write. Field flags change only the given fields; the rest of the meta is kept as it is. Clear a field with \`--from-file\`\: fields the file omits are cleared' \
'package-meta:Show a package'\''s meta (title, description, persons, devel project)' \
'set-package-meta:Set a package'\''s meta, field by field or whole from a file. A mutation\: dry-run is the default (only the plan is printed); \`--live\` performs the write. Field flags change only the given fields; the rest of the meta is kept as it is. Clear a field with \`--from-file\`\: fields the file omits are cleared' \
'update-package-meta-from-spec:Update a package'\''s meta from its spec file. A mutation\: dry-run is the default (only the plan is printed); \`--live\` performs the update. The spec file is found by listing the package files when exactly one \`*.spec\` exists; otherwise name it with \`--spec\`' \
'search-packages:Search packages by name. Substring matches on name, title, and description by default; \`--exact\` keeps only exact-name matches (the filter runs client-side). With \`--devel-project\`, only packages developed in that project; without a term, every package the devel project develops' \
'search-projects:Search projects by name. Substring matches on name, title, and description by default; \`--exact\` keeps only exact-name matches (the filter runs client-side)' \
'search-projects-by-person:Search projects by a person'\''s login\: every project whose meta names the login in a role' \
'search-packages-by-person:Search packages by a person'\''s login\: every package whose meta names the login in a role' \
'count-scmsync-projects:Count the projects syncing from a git URL prefix' \
'list-scmsync-projects:List the projects syncing from a git URL prefix' \
'owners:Show the owners of a package (project/package scope)' \
'owners-for-binary:Show the owners of a binary package' \
'owners-for-package:Show the owners of a package by name alone, in the owner root projects' \
'maintainers:Show the maintainers of a package' \
'whois:Show a user'\''s full name, email, and account state. Without a login, the authenticated user' \
'link-target:Show the link target of a package, when it is a link' \
'project-diff:Diff two projects, package by package. Computed client-side\: the library lists both projects and reads every file of every differing package, so a large diff issues many requests and a differing binary file fails the call' \
'diff-packages:Diff two packages, file by file. Computed client-side\: the library diffs the two file lists and cats each changed file, so a differing binary file fails the call' \
'devel-project:Show the devel project and devel package of a package, when it has one' \
'repo-add:Add a repository to a project. A mutation\: dry-run is the default (only the plan is printed); \`--live\` performs the addition' \
'repo-remove:Remove a repository from a project. A mutation\: dry-run is the default (only the plan is printed); \`--live\` performs the removal. Removing a repository that is not there succeeds' \
'upload-file:Upload a file into a package. A mutation\: dry-run is the default (only the plan is printed); \`--live\` performs the upload. The local file is read from \`--from\`; \`--binary\` uploads bytes instead of text' \
'create-project:Create a project. A mutation\: dry-run is the default (only the plan is printed); \`--live\` performs the creation' \
'create-package:Create a package. A mutation\: dry-run is the default (only the plan is printed); \`--live\` performs the creation. Without \`--scmsync\` and \`--force\` the package is created directly; with either, the creation goes through the options form, which refuses an existing package unless \`--force\` is set and refuses a project managed in git' \
'create-token:Create an operation-scoped trigger token for the authenticated user. A mutation\: dry-run is the default (only the plan is printed); \`--live\` performs the creation. The secret prints once, at creation\: store it now. Project and package go together\: give both to scope the token to a package, or neither for a global token' \
'delete-package:Delete a package. A mutation\: dry-run is the default (only the plan is printed); \`--live\` performs the deletion. Without \`--force\` a package with open submit requests is refused' \
'delete-link:Delete a \`_link\` file from a package. A mutation\: dry-run is the default (only the plan is printed); \`--live\` performs the deletion. The package keeps its files and meta, but stops tracking the source' \
'delete-file:Delete a file from a package. A mutation\: dry-run is the default (only the plan is printed); \`--live\` performs the deletion. The general form of \`delete-link\`\: deleting \`_link\` here issues the same delete as \`delete-link\`. Deleting a file that is already gone is refused by the server (NotFound). The server records the delete with the fixed comment \`removed <file>\`; there is no \`--message\` option, unlike \`delete-package\` — the asymmetry is intentional' \
'delete-project:Delete a project. A mutation\: dry-run is the default (only the plan is printed); \`--live\` performs the deletion. Without \`--recursive\` a project holding packages is refused' \
'undelete-package:Restore a deleted package. A mutation\: dry-run is the default (only the plan is printed); \`--live\` performs the restore' \
'undelete-project:Restore a deleted project. A mutation\: dry-run is the default (only the plan is printed); \`--live\` performs the restore' \
'aggregate-package:Aggregate a package'\''s binaries into another package. A mutation\: dry-run is the default (only the plan is printed); \`--live\` performs the aggregation. The target package is created from the source package'\''s meta when it does not exist yet; an \`_aggregate\` file that already exists is refused' \
'release-package:Release a package'\''s sources and binaries. A mutation\: dry-run is the default (only the plan is printed); \`--live\` performs the release. A release to a target project needs a repository and a target repository' \
'release-project:Release a whole project'\''s sources and binaries. A mutation\: dry-run is the default (only the plan is printed); \`--live\` performs the release. A release to a target project needs a repository and a target repository' \
'lock:Lock a project or a package. A mutation\: dry-run is the default (only the plan is printed); \`--live\` performs the lock. A project lock records the message on the lock; a package lock sends it but the server does not record it' \
'unlock:Unlock a project or a package. A mutation\: dry-run is the default (only the plan is printed); \`--live\` performs the unlock. The message is required\: the server refuses an unlock without a comment' \
'add-channels:Add the channels defined for a project or a package. A mutation\: dry-run is the default (only the plan is printed); \`--live\` performs the addition. A project or package with no channels defined is a no-op' \
'add-containers:Add the maintained containers for a package. A mutation\: dry-run is the default (only the plan is printed); \`--live\` performs the addition' \
    )
    _describe -t commands 'geectl help remote commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__remote__subcmd__add-channels_commands] )) ||
_geectl__subcmd__help__subcmd__remote__subcmd__add-channels_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help remote add-channels commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__remote__subcmd__add-containers_commands] )) ||
_geectl__subcmd__help__subcmd__remote__subcmd__add-containers_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help remote add-containers commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__remote__subcmd__aggregate-package_commands] )) ||
_geectl__subcmd__help__subcmd__remote__subcmd__aggregate-package_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help remote aggregate-package commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__remote__subcmd__cat_commands] )) ||
_geectl__subcmd__help__subcmd__remote__subcmd__cat_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help remote cat commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__remote__subcmd__count-scmsync-projects_commands] )) ||
_geectl__subcmd__help__subcmd__remote__subcmd__count-scmsync-projects_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help remote count-scmsync-projects commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__remote__subcmd__create-package_commands] )) ||
_geectl__subcmd__help__subcmd__remote__subcmd__create-package_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help remote create-package commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__remote__subcmd__create-project_commands] )) ||
_geectl__subcmd__help__subcmd__remote__subcmd__create-project_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help remote create-project commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__remote__subcmd__create-token_commands] )) ||
_geectl__subcmd__help__subcmd__remote__subcmd__create-token_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help remote create-token commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__remote__subcmd__delete-file_commands] )) ||
_geectl__subcmd__help__subcmd__remote__subcmd__delete-file_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help remote delete-file commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__remote__subcmd__delete-link_commands] )) ||
_geectl__subcmd__help__subcmd__remote__subcmd__delete-link_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help remote delete-link commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__remote__subcmd__delete-package_commands] )) ||
_geectl__subcmd__help__subcmd__remote__subcmd__delete-package_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help remote delete-package commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__remote__subcmd__delete-project_commands] )) ||
_geectl__subcmd__help__subcmd__remote__subcmd__delete-project_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help remote delete-project commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__remote__subcmd__devel-project_commands] )) ||
_geectl__subcmd__help__subcmd__remote__subcmd__devel-project_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help remote devel-project commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__remote__subcmd__diff-packages_commands] )) ||
_geectl__subcmd__help__subcmd__remote__subcmd__diff-packages_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help remote diff-packages commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__remote__subcmd__download_commands] )) ||
_geectl__subcmd__help__subcmd__remote__subcmd__download_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help remote download commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__remote__subcmd__link-target_commands] )) ||
_geectl__subcmd__help__subcmd__remote__subcmd__link-target_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help remote link-target commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__remote__subcmd__list-files_commands] )) ||
_geectl__subcmd__help__subcmd__remote__subcmd__list-files_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help remote list-files commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__remote__subcmd__list-packages_commands] )) ||
_geectl__subcmd__help__subcmd__remote__subcmd__list-packages_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help remote list-packages commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__remote__subcmd__list-projects_commands] )) ||
_geectl__subcmd__help__subcmd__remote__subcmd__list-projects_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help remote list-projects commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__remote__subcmd__list-scmsync-projects_commands] )) ||
_geectl__subcmd__help__subcmd__remote__subcmd__list-scmsync-projects_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help remote list-scmsync-projects commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__remote__subcmd__list-staging-projects_commands] )) ||
_geectl__subcmd__help__subcmd__remote__subcmd__list-staging-projects_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help remote list-staging-projects commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__remote__subcmd__lock_commands] )) ||
_geectl__subcmd__help__subcmd__remote__subcmd__lock_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help remote lock commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__remote__subcmd__maintainers_commands] )) ||
_geectl__subcmd__help__subcmd__remote__subcmd__maintainers_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help remote maintainers commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__remote__subcmd__owners_commands] )) ||
_geectl__subcmd__help__subcmd__remote__subcmd__owners_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help remote owners commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__remote__subcmd__owners-for-binary_commands] )) ||
_geectl__subcmd__help__subcmd__remote__subcmd__owners-for-binary_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help remote owners-for-binary commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__remote__subcmd__owners-for-package_commands] )) ||
_geectl__subcmd__help__subcmd__remote__subcmd__owners-for-package_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help remote owners-for-package commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__remote__subcmd__package-log_commands] )) ||
_geectl__subcmd__help__subcmd__remote__subcmd__package-log_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help remote package-log commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__remote__subcmd__package-meta_commands] )) ||
_geectl__subcmd__help__subcmd__remote__subcmd__package-meta_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help remote package-meta commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__remote__subcmd__project-diff_commands] )) ||
_geectl__subcmd__help__subcmd__remote__subcmd__project-diff_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help remote project-diff commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__remote__subcmd__project-meta_commands] )) ||
_geectl__subcmd__help__subcmd__remote__subcmd__project-meta_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help remote project-meta commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__remote__subcmd__release-package_commands] )) ||
_geectl__subcmd__help__subcmd__remote__subcmd__release-package_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help remote release-package commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__remote__subcmd__release-project_commands] )) ||
_geectl__subcmd__help__subcmd__remote__subcmd__release-project_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help remote release-project commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__remote__subcmd__repo-add_commands] )) ||
_geectl__subcmd__help__subcmd__remote__subcmd__repo-add_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help remote repo-add commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__remote__subcmd__repo-remove_commands] )) ||
_geectl__subcmd__help__subcmd__remote__subcmd__repo-remove_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help remote repo-remove commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__remote__subcmd__revision-patch_commands] )) ||
_geectl__subcmd__help__subcmd__remote__subcmd__revision-patch_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help remote revision-patch commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__remote__subcmd__search-packages_commands] )) ||
_geectl__subcmd__help__subcmd__remote__subcmd__search-packages_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help remote search-packages commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__remote__subcmd__search-packages-by-person_commands] )) ||
_geectl__subcmd__help__subcmd__remote__subcmd__search-packages-by-person_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help remote search-packages-by-person commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__remote__subcmd__search-projects_commands] )) ||
_geectl__subcmd__help__subcmd__remote__subcmd__search-projects_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help remote search-projects commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__remote__subcmd__search-projects-by-person_commands] )) ||
_geectl__subcmd__help__subcmd__remote__subcmd__search-projects-by-person_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help remote search-projects-by-person commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__remote__subcmd__set-package-meta_commands] )) ||
_geectl__subcmd__help__subcmd__remote__subcmd__set-package-meta_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help remote set-package-meta commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__remote__subcmd__set-project-meta_commands] )) ||
_geectl__subcmd__help__subcmd__remote__subcmd__set-project-meta_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help remote set-project-meta commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__remote__subcmd__staging-project_commands] )) ||
_geectl__subcmd__help__subcmd__remote__subcmd__staging-project_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help remote staging-project commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__remote__subcmd__undelete-package_commands] )) ||
_geectl__subcmd__help__subcmd__remote__subcmd__undelete-package_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help remote undelete-package commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__remote__subcmd__undelete-project_commands] )) ||
_geectl__subcmd__help__subcmd__remote__subcmd__undelete-project_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help remote undelete-project commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__remote__subcmd__unlock_commands] )) ||
_geectl__subcmd__help__subcmd__remote__subcmd__unlock_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help remote unlock commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__remote__subcmd__update-package-meta-from-spec_commands] )) ||
_geectl__subcmd__help__subcmd__remote__subcmd__update-package-meta-from-spec_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help remote update-package-meta-from-spec commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__remote__subcmd__upload-file_commands] )) ||
_geectl__subcmd__help__subcmd__remote__subcmd__upload-file_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help remote upload-file commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__remote__subcmd__whois_commands] )) ||
_geectl__subcmd__help__subcmd__remote__subcmd__whois_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help remote whois commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__request_commands] )) ||
_geectl__subcmd__help__subcmd__request_commands() {
    local commands; commands=(
'show:Show one submit request\: source/target/message, reviews, history' \
'log:Show one submit request'\''s history\: creation, then each state change' \
'search:List the submit requests matching the filters\: a typed search' \
'list:List submit requests\: the same search, with the states still awaiting action (new, review) as the default. A thin wrapper over the search operation\: the flags select the same server-side search' \
'diff:Show the diff of a submit request\: the unified diff, or the request XML with the diff injected with \`--xml\`' \
'list-comments:List the comments on a submit request, in id order' \
'accept:Accept a submit request, with a comment as the message left with the accept. An accepted request is forwarded to its link parent unless \`--no-forward\` is given; a refused forward is reported and the accepted request stands, with the work staying queued in the devel project. The forwarded request'\''s \`.changes\` entry is the original description, overridable with \`-m\` / \`--forward-message\`. Dry-run by default; mutates only with \`--live\`' \
'decline:Decline a submit request, with a comment as the reason' \
'revoke:Revoke (withdraw) a submit request, with a comment as the reason' \
'change-state:Change a submit request'\''s state to one of new/review/accepted/declined/revoked' \
'try-change-state:Best-effort state change\: try it, and report whether it happened' \
'add-review:Ask a user, group, project, or package to review a submit request' \
'change-review-state:Change the state of one review on a submit request\: accept it, decline it, or reopen it to new' \
'add-comment:Comment on a submit request, optionally as a reply to another comment' \
'file:File a submit request\: source package to a devel project, superseding every open submit request for the package from the same user' \
'refile:File a submit request and supersede an old one with it, plus every other open submit request for the package from the same user\: the new request is filed first, and only then is each open one superseded pointing at it' \
'supersede:Supersede one request with another existing one. A mutation that needs \`--live\`; the dry-runnable supersede flow is \`request refile --supersede\`' \
'file-add-role:File an add-role request\: grant a person or a group a role on a project or a package' \
'file-change-devel:File a change-devel request\: change a package'\''s devel project' \
'file-delete:File a delete request\: delete a package, a repository, or a whole project' \
'file-maintenance-incident:File a maintenance incident request\: ask the maintenance team to start a maintenance incident from a maintenance update' \
'refile-maintenance-incident:File a maintenance incident request and supersede an old one with it\: the new request is filed first, and only then is the old one superseded pointing at it' \
    )
    _describe -t commands 'geectl help request commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__request__subcmd__accept_commands] )) ||
_geectl__subcmd__help__subcmd__request__subcmd__accept_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help request accept commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__request__subcmd__add-comment_commands] )) ||
_geectl__subcmd__help__subcmd__request__subcmd__add-comment_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help request add-comment commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__request__subcmd__add-review_commands] )) ||
_geectl__subcmd__help__subcmd__request__subcmd__add-review_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help request add-review commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__request__subcmd__change-review-state_commands] )) ||
_geectl__subcmd__help__subcmd__request__subcmd__change-review-state_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help request change-review-state commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__request__subcmd__change-state_commands] )) ||
_geectl__subcmd__help__subcmd__request__subcmd__change-state_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help request change-state commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__request__subcmd__decline_commands] )) ||
_geectl__subcmd__help__subcmd__request__subcmd__decline_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help request decline commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__request__subcmd__diff_commands] )) ||
_geectl__subcmd__help__subcmd__request__subcmd__diff_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help request diff commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__request__subcmd__file_commands] )) ||
_geectl__subcmd__help__subcmd__request__subcmd__file_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help request file commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__request__subcmd__file-add-role_commands] )) ||
_geectl__subcmd__help__subcmd__request__subcmd__file-add-role_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help request file-add-role commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__request__subcmd__file-change-devel_commands] )) ||
_geectl__subcmd__help__subcmd__request__subcmd__file-change-devel_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help request file-change-devel commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__request__subcmd__file-delete_commands] )) ||
_geectl__subcmd__help__subcmd__request__subcmd__file-delete_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help request file-delete commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__request__subcmd__file-maintenance-incident_commands] )) ||
_geectl__subcmd__help__subcmd__request__subcmd__file-maintenance-incident_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help request file-maintenance-incident commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__request__subcmd__list_commands] )) ||
_geectl__subcmd__help__subcmd__request__subcmd__list_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help request list commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__request__subcmd__list-comments_commands] )) ||
_geectl__subcmd__help__subcmd__request__subcmd__list-comments_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help request list-comments commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__request__subcmd__log_commands] )) ||
_geectl__subcmd__help__subcmd__request__subcmd__log_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help request log commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__request__subcmd__refile_commands] )) ||
_geectl__subcmd__help__subcmd__request__subcmd__refile_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help request refile commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__request__subcmd__refile-maintenance-incident_commands] )) ||
_geectl__subcmd__help__subcmd__request__subcmd__refile-maintenance-incident_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help request refile-maintenance-incident commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__request__subcmd__revoke_commands] )) ||
_geectl__subcmd__help__subcmd__request__subcmd__revoke_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help request revoke commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__request__subcmd__search_commands] )) ||
_geectl__subcmd__help__subcmd__request__subcmd__search_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help request search commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__request__subcmd__show_commands] )) ||
_geectl__subcmd__help__subcmd__request__subcmd__show_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help request show commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__request__subcmd__supersede_commands] )) ||
_geectl__subcmd__help__subcmd__request__subcmd__supersede_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help request supersede commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__request__subcmd__try-change-state_commands] )) ||
_geectl__subcmd__help__subcmd__request__subcmd__try-change-state_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help request try-change-state commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__resolve_commands] )) ||
_geectl__subcmd__help__subcmd__resolve_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help resolve commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__rm_commands] )) ||
_geectl__subcmd__help__subcmd__rm_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help rm commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__service_commands] )) ||
_geectl__subcmd__help__subcmd__service_commands() {
    local commands; commands=(
'list:List the services defined in a package'\''s \`_service\` file' \
'state:Show the last service run state of a package' \
'trigger:Trigger a server-side run of a package'\''s services. A mutation\: dry-run is the default (only the plan is printed); \`--live\` performs the trigger. The trigger only queues the run\: the server runs the services asynchronously, so a successful trigger says the run was queued. Check the outcome with \`service state\`' \
    )
    _describe -t commands 'geectl help service commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__service__subcmd__list_commands] )) ||
_geectl__subcmd__help__subcmd__service__subcmd__list_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help service list commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__service__subcmd__state_commands] )) ||
_geectl__subcmd__help__subcmd__service__subcmd__state_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help service state commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__service__subcmd__trigger_commands] )) ||
_geectl__subcmd__help__subcmd__service__subcmd__trigger_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help service trigger commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__status_commands] )) ||
_geectl__subcmd__help__subcmd__status_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help status commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__update_commands] )) ||
_geectl__subcmd__help__subcmd__update_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help update commands' commands "$@"
}
(( $+functions[_geectl__subcmd__help__subcmd__version_commands] )) ||
_geectl__subcmd__help__subcmd__version_commands() {
    local commands; commands=()
    _describe -t commands 'geectl help version commands' commands "$@"
}
(( $+functions[_geectl__subcmd__local-build_commands] )) ||
_geectl__subcmd__local-build_commands() {
    local commands; commands=(
'service-run:Run the \`_service\` file'\''s services in the container, filtered by the run mode' \
'precommit:The precommit gate\: the \`precommit\`-mode service run, then \`pre_checkin.sh\` when the source directory carries one' \
'spec-cleaner:Run \`spec-cleaner\` on a spec in the container' \
'build:Build the package in the container\: the run-mode service run, the precommit gate, then the engine build' \
'wipe:Wipe a build root\: delete its contents (the lock file stays). A mutation\: dry-run is the default (only the plan is printed); \`--live\` performs the wipe. The marker check and the in-use lock are safety invariants, never bypassed' \
'prune:Print the eviction plan (what would be evicted, in order, with bytes); delete only with \`--apply\`. The plan is the default\: the leaf'\''s own \`--apply\` is the confirmation, so the scheduled task runs exactly \`prune --apply\`' \
'periodic-install:Install the daily periodic cleanup for this platform\: write the scheduler entry and enable it. A mutation\: dry-run is the default (only the plan is printed); \`--live\` performs the install' \
'periodic-uninstall:Remove the daily periodic cleanup for this platform\: disable the scheduler entry, then remove its files. A mutation\: dry-run is the default (only the plan is printed); \`--live\` performs the uninstall' \
'periodic-status:Show whether the daily periodic cleanup is installed and enabled on this platform. Read-only' \
'chroot:Enter the preserved build root of the last build for the package and run repro commands there, read from stdin, one per line, joined with \`&&\` into one shell command. A mutation\: dry-run is the default (only the plan is printed); \`--live\` runs the commands in the root. A terminal on stdin is refused instead of blocking on a read that never arrives. A failing repro still exits 0\: the exit code is data, in \`exit\:\` and in \`record.result.exit_code\`' \
'log:Read the log of the last local build\: the \`<root>/.build.log\` the build'\''s coordinates name. A read\: no \`--live\` needed. The text is the log verbatim (design rule 4); the verdict, exit code, log path, and truncation flag ride in the JSON payload' \
'help:Print this message or the help of the given subcommand(s)' \
    )
    _describe -t commands 'geectl local-build commands' commands "$@"
}
(( $+functions[_geectl__subcmd__local-build__subcmd__build_commands] )) ||
_geectl__subcmd__local-build__subcmd__build_commands() {
    local commands; commands=()
    _describe -t commands 'geectl local-build build commands' commands "$@"
}
(( $+functions[_geectl__subcmd__local-build__subcmd__chroot_commands] )) ||
_geectl__subcmd__local-build__subcmd__chroot_commands() {
    local commands; commands=()
    _describe -t commands 'geectl local-build chroot commands' commands "$@"
}
(( $+functions[_geectl__subcmd__local-build__subcmd__help_commands] )) ||
_geectl__subcmd__local-build__subcmd__help_commands() {
    local commands; commands=(
'service-run:Run the \`_service\` file'\''s services in the container, filtered by the run mode' \
'precommit:The precommit gate\: the \`precommit\`-mode service run, then \`pre_checkin.sh\` when the source directory carries one' \
'spec-cleaner:Run \`spec-cleaner\` on a spec in the container' \
'build:Build the package in the container\: the run-mode service run, the precommit gate, then the engine build' \
'wipe:Wipe a build root\: delete its contents (the lock file stays). A mutation\: dry-run is the default (only the plan is printed); \`--live\` performs the wipe. The marker check and the in-use lock are safety invariants, never bypassed' \
'prune:Print the eviction plan (what would be evicted, in order, with bytes); delete only with \`--apply\`. The plan is the default\: the leaf'\''s own \`--apply\` is the confirmation, so the scheduled task runs exactly \`prune --apply\`' \
'periodic-install:Install the daily periodic cleanup for this platform\: write the scheduler entry and enable it. A mutation\: dry-run is the default (only the plan is printed); \`--live\` performs the install' \
'periodic-uninstall:Remove the daily periodic cleanup for this platform\: disable the scheduler entry, then remove its files. A mutation\: dry-run is the default (only the plan is printed); \`--live\` performs the uninstall' \
'periodic-status:Show whether the daily periodic cleanup is installed and enabled on this platform. Read-only' \
'chroot:Enter the preserved build root of the last build for the package and run repro commands there, read from stdin, one per line, joined with \`&&\` into one shell command. A mutation\: dry-run is the default (only the plan is printed); \`--live\` runs the commands in the root. A terminal on stdin is refused instead of blocking on a read that never arrives. A failing repro still exits 0\: the exit code is data, in \`exit\:\` and in \`record.result.exit_code\`' \
'log:Read the log of the last local build\: the \`<root>/.build.log\` the build'\''s coordinates name. A read\: no \`--live\` needed. The text is the log verbatim (design rule 4); the verdict, exit code, log path, and truncation flag ride in the JSON payload' \
'help:Print this message or the help of the given subcommand(s)' \
    )
    _describe -t commands 'geectl local-build help commands' commands "$@"
}
(( $+functions[_geectl__subcmd__local-build__subcmd__help__subcmd__build_commands] )) ||
_geectl__subcmd__local-build__subcmd__help__subcmd__build_commands() {
    local commands; commands=()
    _describe -t commands 'geectl local-build help build commands' commands "$@"
}
(( $+functions[_geectl__subcmd__local-build__subcmd__help__subcmd__chroot_commands] )) ||
_geectl__subcmd__local-build__subcmd__help__subcmd__chroot_commands() {
    local commands; commands=()
    _describe -t commands 'geectl local-build help chroot commands' commands "$@"
}
(( $+functions[_geectl__subcmd__local-build__subcmd__help__subcmd__help_commands] )) ||
_geectl__subcmd__local-build__subcmd__help__subcmd__help_commands() {
    local commands; commands=()
    _describe -t commands 'geectl local-build help help commands' commands "$@"
}
(( $+functions[_geectl__subcmd__local-build__subcmd__help__subcmd__log_commands] )) ||
_geectl__subcmd__local-build__subcmd__help__subcmd__log_commands() {
    local commands; commands=()
    _describe -t commands 'geectl local-build help log commands' commands "$@"
}
(( $+functions[_geectl__subcmd__local-build__subcmd__help__subcmd__periodic-install_commands] )) ||
_geectl__subcmd__local-build__subcmd__help__subcmd__periodic-install_commands() {
    local commands; commands=()
    _describe -t commands 'geectl local-build help periodic-install commands' commands "$@"
}
(( $+functions[_geectl__subcmd__local-build__subcmd__help__subcmd__periodic-status_commands] )) ||
_geectl__subcmd__local-build__subcmd__help__subcmd__periodic-status_commands() {
    local commands; commands=()
    _describe -t commands 'geectl local-build help periodic-status commands' commands "$@"
}
(( $+functions[_geectl__subcmd__local-build__subcmd__help__subcmd__periodic-uninstall_commands] )) ||
_geectl__subcmd__local-build__subcmd__help__subcmd__periodic-uninstall_commands() {
    local commands; commands=()
    _describe -t commands 'geectl local-build help periodic-uninstall commands' commands "$@"
}
(( $+functions[_geectl__subcmd__local-build__subcmd__help__subcmd__precommit_commands] )) ||
_geectl__subcmd__local-build__subcmd__help__subcmd__precommit_commands() {
    local commands; commands=()
    _describe -t commands 'geectl local-build help precommit commands' commands "$@"
}
(( $+functions[_geectl__subcmd__local-build__subcmd__help__subcmd__prune_commands] )) ||
_geectl__subcmd__local-build__subcmd__help__subcmd__prune_commands() {
    local commands; commands=()
    _describe -t commands 'geectl local-build help prune commands' commands "$@"
}
(( $+functions[_geectl__subcmd__local-build__subcmd__help__subcmd__service-run_commands] )) ||
_geectl__subcmd__local-build__subcmd__help__subcmd__service-run_commands() {
    local commands; commands=()
    _describe -t commands 'geectl local-build help service-run commands' commands "$@"
}
(( $+functions[_geectl__subcmd__local-build__subcmd__help__subcmd__spec-cleaner_commands] )) ||
_geectl__subcmd__local-build__subcmd__help__subcmd__spec-cleaner_commands() {
    local commands; commands=()
    _describe -t commands 'geectl local-build help spec-cleaner commands' commands "$@"
}
(( $+functions[_geectl__subcmd__local-build__subcmd__help__subcmd__wipe_commands] )) ||
_geectl__subcmd__local-build__subcmd__help__subcmd__wipe_commands() {
    local commands; commands=()
    _describe -t commands 'geectl local-build help wipe commands' commands "$@"
}
(( $+functions[_geectl__subcmd__local-build__subcmd__log_commands] )) ||
_geectl__subcmd__local-build__subcmd__log_commands() {
    local commands; commands=()
    _describe -t commands 'geectl local-build log commands' commands "$@"
}
(( $+functions[_geectl__subcmd__local-build__subcmd__periodic-install_commands] )) ||
_geectl__subcmd__local-build__subcmd__periodic-install_commands() {
    local commands; commands=()
    _describe -t commands 'geectl local-build periodic-install commands' commands "$@"
}
(( $+functions[_geectl__subcmd__local-build__subcmd__periodic-status_commands] )) ||
_geectl__subcmd__local-build__subcmd__periodic-status_commands() {
    local commands; commands=()
    _describe -t commands 'geectl local-build periodic-status commands' commands "$@"
}
(( $+functions[_geectl__subcmd__local-build__subcmd__periodic-uninstall_commands] )) ||
_geectl__subcmd__local-build__subcmd__periodic-uninstall_commands() {
    local commands; commands=()
    _describe -t commands 'geectl local-build periodic-uninstall commands' commands "$@"
}
(( $+functions[_geectl__subcmd__local-build__subcmd__precommit_commands] )) ||
_geectl__subcmd__local-build__subcmd__precommit_commands() {
    local commands; commands=()
    _describe -t commands 'geectl local-build precommit commands' commands "$@"
}
(( $+functions[_geectl__subcmd__local-build__subcmd__prune_commands] )) ||
_geectl__subcmd__local-build__subcmd__prune_commands() {
    local commands; commands=()
    _describe -t commands 'geectl local-build prune commands' commands "$@"
}
(( $+functions[_geectl__subcmd__local-build__subcmd__service-run_commands] )) ||
_geectl__subcmd__local-build__subcmd__service-run_commands() {
    local commands; commands=()
    _describe -t commands 'geectl local-build service-run commands' commands "$@"
}
(( $+functions[_geectl__subcmd__local-build__subcmd__spec-cleaner_commands] )) ||
_geectl__subcmd__local-build__subcmd__spec-cleaner_commands() {
    local commands; commands=()
    _describe -t commands 'geectl local-build spec-cleaner commands' commands "$@"
}
(( $+functions[_geectl__subcmd__local-build__subcmd__wipe_commands] )) ||
_geectl__subcmd__local-build__subcmd__wipe_commands() {
    local commands; commands=()
    _describe -t commands 'geectl local-build wipe commands' commands "$@"
}
(( $+functions[_geectl__subcmd__maintenance_commands] )) ||
_geectl__subcmd__maintenance_commands() {
    local commands; commands=(
'maintained-instances:List the maintained instances of a package\: the projects carrying the OBS\:Maintained attribute that the package exists in, with where a branch would land. Read-only; requires OBS credentials' \
'maintenance-branch:Branch a maintained package into its maintenance branch project and report where it landed. A mutation\: dry-run is the default (only the plan is printed); the global \`--live\` performs the branch' \
'help:Print this message or the help of the given subcommand(s)' \
    )
    _describe -t commands 'geectl maintenance commands' commands "$@"
}
(( $+functions[_geectl__subcmd__maintenance__subcmd__help_commands] )) ||
_geectl__subcmd__maintenance__subcmd__help_commands() {
    local commands; commands=(
'maintained-instances:List the maintained instances of a package\: the projects carrying the OBS\:Maintained attribute that the package exists in, with where a branch would land. Read-only; requires OBS credentials' \
'maintenance-branch:Branch a maintained package into its maintenance branch project and report where it landed. A mutation\: dry-run is the default (only the plan is printed); the global \`--live\` performs the branch' \
'help:Print this message or the help of the given subcommand(s)' \
    )
    _describe -t commands 'geectl maintenance help commands' commands "$@"
}
(( $+functions[_geectl__subcmd__maintenance__subcmd__help__subcmd__help_commands] )) ||
_geectl__subcmd__maintenance__subcmd__help__subcmd__help_commands() {
    local commands; commands=()
    _describe -t commands 'geectl maintenance help help commands' commands "$@"
}
(( $+functions[_geectl__subcmd__maintenance__subcmd__help__subcmd__maintained-instances_commands] )) ||
_geectl__subcmd__maintenance__subcmd__help__subcmd__maintained-instances_commands() {
    local commands; commands=()
    _describe -t commands 'geectl maintenance help maintained-instances commands' commands "$@"
}
(( $+functions[_geectl__subcmd__maintenance__subcmd__help__subcmd__maintenance-branch_commands] )) ||
_geectl__subcmd__maintenance__subcmd__help__subcmd__maintenance-branch_commands() {
    local commands; commands=()
    _describe -t commands 'geectl maintenance help maintenance-branch commands' commands "$@"
}
(( $+functions[_geectl__subcmd__maintenance__subcmd__maintained-instances_commands] )) ||
_geectl__subcmd__maintenance__subcmd__maintained-instances_commands() {
    local commands; commands=()
    _describe -t commands 'geectl maintenance maintained-instances commands' commands "$@"
}
(( $+functions[_geectl__subcmd__maintenance__subcmd__maintenance-branch_commands] )) ||
_geectl__subcmd__maintenance__subcmd__maintenance-branch_commands() {
    local commands; commands=()
    _describe -t commands 'geectl maintenance maintenance-branch commands' commands "$@"
}
(( $+functions[_geectl__subcmd__meta_commands] )) ||
_geectl__subcmd__meta_commands() {
    local commands; commands=(
'set:Set the meta of an OBS project, an OBS package, or a Gitea repository\: the description and/or URL. The target selects the backend\: \`--project\` (with \`--package\` for a package) writes OBS meta, \`--repo\` writes Gitea repo settings. A mutation\: dry-run is the default (only the plan is printed); \`--live\` performs the write. Only the given fields change; the rest of the meta is kept as it is. There is no flag to clear a field' \
'help:Print this message or the help of the given subcommand(s)' \
    )
    _describe -t commands 'geectl meta commands' commands "$@"
}
(( $+functions[_geectl__subcmd__meta__subcmd__help_commands] )) ||
_geectl__subcmd__meta__subcmd__help_commands() {
    local commands; commands=(
'set:Set the meta of an OBS project, an OBS package, or a Gitea repository\: the description and/or URL. The target selects the backend\: \`--project\` (with \`--package\` for a package) writes OBS meta, \`--repo\` writes Gitea repo settings. A mutation\: dry-run is the default (only the plan is printed); \`--live\` performs the write. Only the given fields change; the rest of the meta is kept as it is. There is no flag to clear a field' \
'help:Print this message or the help of the given subcommand(s)' \
    )
    _describe -t commands 'geectl meta help commands' commands "$@"
}
(( $+functions[_geectl__subcmd__meta__subcmd__help__subcmd__help_commands] )) ||
_geectl__subcmd__meta__subcmd__help__subcmd__help_commands() {
    local commands; commands=()
    _describe -t commands 'geectl meta help help commands' commands "$@"
}
(( $+functions[_geectl__subcmd__meta__subcmd__help__subcmd__set_commands] )) ||
_geectl__subcmd__meta__subcmd__help__subcmd__set_commands() {
    local commands; commands=()
    _describe -t commands 'geectl meta help set commands' commands "$@"
}
(( $+functions[_geectl__subcmd__meta__subcmd__set_commands] )) ||
_geectl__subcmd__meta__subcmd__set_commands() {
    local commands; commands=()
    _describe -t commands 'geectl meta set commands' commands "$@"
}
(( $+functions[_geectl__subcmd__person_commands] )) ||
_geectl__subcmd__person_commands() {
    local commands; commands=(
'search:Search for a person\: the OBS projects naming the login, and the Gitea users matching the keyword. With no \`--backend\` both arms run, OBS first then Gitea' \
'help:Print this message or the help of the given subcommand(s)' \
    )
    _describe -t commands 'geectl person commands' commands "$@"
}
(( $+functions[_geectl__subcmd__person__subcmd__help_commands] )) ||
_geectl__subcmd__person__subcmd__help_commands() {
    local commands; commands=(
'search:Search for a person\: the OBS projects naming the login, and the Gitea users matching the keyword. With no \`--backend\` both arms run, OBS first then Gitea' \
'help:Print this message or the help of the given subcommand(s)' \
    )
    _describe -t commands 'geectl person help commands' commands "$@"
}
(( $+functions[_geectl__subcmd__person__subcmd__help__subcmd__help_commands] )) ||
_geectl__subcmd__person__subcmd__help__subcmd__help_commands() {
    local commands; commands=()
    _describe -t commands 'geectl person help help commands' commands "$@"
}
(( $+functions[_geectl__subcmd__person__subcmd__help__subcmd__search_commands] )) ||
_geectl__subcmd__person__subcmd__help__subcmd__search_commands() {
    local commands; commands=()
    _describe -t commands 'geectl person help search commands' commands "$@"
}
(( $+functions[_geectl__subcmd__person__subcmd__search_commands] )) ||
_geectl__subcmd__person__subcmd__search_commands() {
    local commands; commands=()
    _describe -t commands 'geectl person search commands' commands "$@"
}
(( $+functions[_geectl__subcmd__prjconf_commands] )) ||
_geectl__subcmd__prjconf_commands() {
    local commands; commands=(
'show:Show a project'\''s raw prjconf' \
'validate:Validate a project'\''s prjconf across its repositories. Exits 24 when error findings exist' \
'clean:Clean a project'\''s prjconf. A mutation\: dry-run is the default. Shares validate'\''s credential gate and 32-repository bound' \
'help:Print this message or the help of the given subcommand(s)' \
    )
    _describe -t commands 'geectl prjconf commands' commands "$@"
}
(( $+functions[_geectl__subcmd__prjconf__subcmd__clean_commands] )) ||
_geectl__subcmd__prjconf__subcmd__clean_commands() {
    local commands; commands=()
    _describe -t commands 'geectl prjconf clean commands' commands "$@"
}
(( $+functions[_geectl__subcmd__prjconf__subcmd__help_commands] )) ||
_geectl__subcmd__prjconf__subcmd__help_commands() {
    local commands; commands=(
'show:Show a project'\''s raw prjconf' \
'validate:Validate a project'\''s prjconf across its repositories. Exits 24 when error findings exist' \
'clean:Clean a project'\''s prjconf. A mutation\: dry-run is the default. Shares validate'\''s credential gate and 32-repository bound' \
'help:Print this message or the help of the given subcommand(s)' \
    )
    _describe -t commands 'geectl prjconf help commands' commands "$@"
}
(( $+functions[_geectl__subcmd__prjconf__subcmd__help__subcmd__clean_commands] )) ||
_geectl__subcmd__prjconf__subcmd__help__subcmd__clean_commands() {
    local commands; commands=()
    _describe -t commands 'geectl prjconf help clean commands' commands "$@"
}
(( $+functions[_geectl__subcmd__prjconf__subcmd__help__subcmd__help_commands] )) ||
_geectl__subcmd__prjconf__subcmd__help__subcmd__help_commands() {
    local commands; commands=()
    _describe -t commands 'geectl prjconf help help commands' commands "$@"
}
(( $+functions[_geectl__subcmd__prjconf__subcmd__help__subcmd__show_commands] )) ||
_geectl__subcmd__prjconf__subcmd__help__subcmd__show_commands() {
    local commands; commands=()
    _describe -t commands 'geectl prjconf help show commands' commands "$@"
}
(( $+functions[_geectl__subcmd__prjconf__subcmd__help__subcmd__validate_commands] )) ||
_geectl__subcmd__prjconf__subcmd__help__subcmd__validate_commands() {
    local commands; commands=()
    _describe -t commands 'geectl prjconf help validate commands' commands "$@"
}
(( $+functions[_geectl__subcmd__prjconf__subcmd__show_commands] )) ||
_geectl__subcmd__prjconf__subcmd__show_commands() {
    local commands; commands=()
    _describe -t commands 'geectl prjconf show commands' commands "$@"
}
(( $+functions[_geectl__subcmd__prjconf__subcmd__validate_commands] )) ||
_geectl__subcmd__prjconf__subcmd__validate_commands() {
    local commands; commands=()
    _describe -t commands 'geectl prjconf validate commands' commands "$@"
}
(( $+functions[_geectl__subcmd__pull_commands] )) ||
_geectl__subcmd__pull_commands() {
    local commands; commands=()
    _describe -t commands 'geectl pull commands' commands "$@"
}
(( $+functions[_geectl__subcmd__pull-request_commands] )) ||
_geectl__subcmd__pull-request_commands() {
    local commands; commands=(
'show:Show one pull request\: \`obs\:<id>\` or \`gitea\:<owner>/<repo>#<number>\`' \
'diff:Show the diff of a pull request\: the unified diff, like \`request diff\`. With \`--xml\` the OBS arm renders the request XML with the diff injected; \`--xml\` is refused on a Gitea pull request, which has no request XML. A read, always live' \
'search:Search pull requests\: OBS submit requests and Gitea pull requests as one list, OBS first then Gitea. With no \`--backend\` both arms run; \`--backend\` scopes to one' \
'approve:Approve a pull request as the review asked of the target\: with no target flag, as the review asked of the acting user (OBS) or the token owner (Gitea). Dry-run by default; mutates only with \`--live\`' \
'request-changes:Ask for changes on a pull request, as \`approve\` but with the review declined instead of accepted. Dry-run by default; mutates only with \`--live\`' \
'comment:Comment on a pull request. Dry-run by default; mutates only with \`--live\`' \
'build-comment:Post the build-link comment on a Gitea pull request\: the OBS build results for the test project and package, as a link plus a one-line per-instance summary. The comment is posted once, on PR opening; there is no polling, no live update, and no edit. Dry-run by default; mutates only with \`--live\`' \
'comments:List the comments on a pull request, oldest first. A read, always live' \
'cancel-review-request:Cancel pending review requests on a pull request. Dry-run by default; mutates only with \`--live\`. On OBS the cancel fails closed' \
'merge:Merge a pull request. On OBS this accepts the submit request and forwards it to its link parent; a refused forward is reported and the accepted request stands. On Gitea the forward is resolved and validated before the merge lands, and the merged pull request is forwarded to its link parent too, unless \`--no-forward\` is given. The forwarded request'\''s \`.changes\` entry is the original description on the OBS arm and the pull request'\''s text on the Gitea arm, overridable with \`-m\` / \`--forward-message\`. Dry-run by default; mutates only with \`--live\`' \
'decline:Decline a pull request. On OBS this changes the request state to declined; on Gitea it closes the pull request. Dry-run by default; mutates only with \`--live\`' \
'withdraw:Withdraw a pull request. On OBS this revokes the request; on Gitea it closes the pull request. Dry-run by default; mutates only with \`--live\`' \
'reopen:Reopen a pull request. On OBS this changes a declined request back to \`new\`; on Gitea it reopens the closed pull request. Dry-run by default; mutates only with \`--live\`' \
'submit:File a submission from the working copy in the current directory\: an OBS submit request from an OBS-native checkout, a Gitea pull request from a git-backed one. The checkout'\''s backend decides; the other arm'\''s flags are refused. Dry-run by default; mutates only with \`--live\`' \
'cleanup:Clean the local build roots for the submit request filed from the working copy in the current directory, when that request is accepted. The request id was recorded by \`submit\` in the checkout; the leaf reads the request state once and wipes the roots only on \`accepted\`. A request in any other state, and a checkout with nothing recorded, delete nothing. Dry-run by default; mutates only with \`--live\`' \
'help:Print this message or the help of the given subcommand(s)' \
    )
    _describe -t commands 'geectl pull-request commands' commands "$@"
}
(( $+functions[_geectl__subcmd__pull-request__subcmd__approve_commands] )) ||
_geectl__subcmd__pull-request__subcmd__approve_commands() {
    local commands; commands=()
    _describe -t commands 'geectl pull-request approve commands' commands "$@"
}
(( $+functions[_geectl__subcmd__pull-request__subcmd__build-comment_commands] )) ||
_geectl__subcmd__pull-request__subcmd__build-comment_commands() {
    local commands; commands=()
    _describe -t commands 'geectl pull-request build-comment commands' commands "$@"
}
(( $+functions[_geectl__subcmd__pull-request__subcmd__cancel-review-request_commands] )) ||
_geectl__subcmd__pull-request__subcmd__cancel-review-request_commands() {
    local commands; commands=()
    _describe -t commands 'geectl pull-request cancel-review-request commands' commands "$@"
}
(( $+functions[_geectl__subcmd__pull-request__subcmd__cleanup_commands] )) ||
_geectl__subcmd__pull-request__subcmd__cleanup_commands() {
    local commands; commands=()
    _describe -t commands 'geectl pull-request cleanup commands' commands "$@"
}
(( $+functions[_geectl__subcmd__pull-request__subcmd__comment_commands] )) ||
_geectl__subcmd__pull-request__subcmd__comment_commands() {
    local commands; commands=()
    _describe -t commands 'geectl pull-request comment commands' commands "$@"
}
(( $+functions[_geectl__subcmd__pull-request__subcmd__comments_commands] )) ||
_geectl__subcmd__pull-request__subcmd__comments_commands() {
    local commands; commands=()
    _describe -t commands 'geectl pull-request comments commands' commands "$@"
}
(( $+functions[_geectl__subcmd__pull-request__subcmd__decline_commands] )) ||
_geectl__subcmd__pull-request__subcmd__decline_commands() {
    local commands; commands=()
    _describe -t commands 'geectl pull-request decline commands' commands "$@"
}
(( $+functions[_geectl__subcmd__pull-request__subcmd__diff_commands] )) ||
_geectl__subcmd__pull-request__subcmd__diff_commands() {
    local commands; commands=()
    _describe -t commands 'geectl pull-request diff commands' commands "$@"
}
(( $+functions[_geectl__subcmd__pull-request__subcmd__help_commands] )) ||
_geectl__subcmd__pull-request__subcmd__help_commands() {
    local commands; commands=(
'show:Show one pull request\: \`obs\:<id>\` or \`gitea\:<owner>/<repo>#<number>\`' \
'diff:Show the diff of a pull request\: the unified diff, like \`request diff\`. With \`--xml\` the OBS arm renders the request XML with the diff injected; \`--xml\` is refused on a Gitea pull request, which has no request XML. A read, always live' \
'search:Search pull requests\: OBS submit requests and Gitea pull requests as one list, OBS first then Gitea. With no \`--backend\` both arms run; \`--backend\` scopes to one' \
'approve:Approve a pull request as the review asked of the target\: with no target flag, as the review asked of the acting user (OBS) or the token owner (Gitea). Dry-run by default; mutates only with \`--live\`' \
'request-changes:Ask for changes on a pull request, as \`approve\` but with the review declined instead of accepted. Dry-run by default; mutates only with \`--live\`' \
'comment:Comment on a pull request. Dry-run by default; mutates only with \`--live\`' \
'build-comment:Post the build-link comment on a Gitea pull request\: the OBS build results for the test project and package, as a link plus a one-line per-instance summary. The comment is posted once, on PR opening; there is no polling, no live update, and no edit. Dry-run by default; mutates only with \`--live\`' \
'comments:List the comments on a pull request, oldest first. A read, always live' \
'cancel-review-request:Cancel pending review requests on a pull request. Dry-run by default; mutates only with \`--live\`. On OBS the cancel fails closed' \
'merge:Merge a pull request. On OBS this accepts the submit request and forwards it to its link parent; a refused forward is reported and the accepted request stands. On Gitea the forward is resolved and validated before the merge lands, and the merged pull request is forwarded to its link parent too, unless \`--no-forward\` is given. The forwarded request'\''s \`.changes\` entry is the original description on the OBS arm and the pull request'\''s text on the Gitea arm, overridable with \`-m\` / \`--forward-message\`. Dry-run by default; mutates only with \`--live\`' \
'decline:Decline a pull request. On OBS this changes the request state to declined; on Gitea it closes the pull request. Dry-run by default; mutates only with \`--live\`' \
'withdraw:Withdraw a pull request. On OBS this revokes the request; on Gitea it closes the pull request. Dry-run by default; mutates only with \`--live\`' \
'reopen:Reopen a pull request. On OBS this changes a declined request back to \`new\`; on Gitea it reopens the closed pull request. Dry-run by default; mutates only with \`--live\`' \
'submit:File a submission from the working copy in the current directory\: an OBS submit request from an OBS-native checkout, a Gitea pull request from a git-backed one. The checkout'\''s backend decides; the other arm'\''s flags are refused. Dry-run by default; mutates only with \`--live\`' \
'cleanup:Clean the local build roots for the submit request filed from the working copy in the current directory, when that request is accepted. The request id was recorded by \`submit\` in the checkout; the leaf reads the request state once and wipes the roots only on \`accepted\`. A request in any other state, and a checkout with nothing recorded, delete nothing. Dry-run by default; mutates only with \`--live\`' \
'help:Print this message or the help of the given subcommand(s)' \
    )
    _describe -t commands 'geectl pull-request help commands' commands "$@"
}
(( $+functions[_geectl__subcmd__pull-request__subcmd__help__subcmd__approve_commands] )) ||
_geectl__subcmd__pull-request__subcmd__help__subcmd__approve_commands() {
    local commands; commands=()
    _describe -t commands 'geectl pull-request help approve commands' commands "$@"
}
(( $+functions[_geectl__subcmd__pull-request__subcmd__help__subcmd__build-comment_commands] )) ||
_geectl__subcmd__pull-request__subcmd__help__subcmd__build-comment_commands() {
    local commands; commands=()
    _describe -t commands 'geectl pull-request help build-comment commands' commands "$@"
}
(( $+functions[_geectl__subcmd__pull-request__subcmd__help__subcmd__cancel-review-request_commands] )) ||
_geectl__subcmd__pull-request__subcmd__help__subcmd__cancel-review-request_commands() {
    local commands; commands=()
    _describe -t commands 'geectl pull-request help cancel-review-request commands' commands "$@"
}
(( $+functions[_geectl__subcmd__pull-request__subcmd__help__subcmd__cleanup_commands] )) ||
_geectl__subcmd__pull-request__subcmd__help__subcmd__cleanup_commands() {
    local commands; commands=()
    _describe -t commands 'geectl pull-request help cleanup commands' commands "$@"
}
(( $+functions[_geectl__subcmd__pull-request__subcmd__help__subcmd__comment_commands] )) ||
_geectl__subcmd__pull-request__subcmd__help__subcmd__comment_commands() {
    local commands; commands=()
    _describe -t commands 'geectl pull-request help comment commands' commands "$@"
}
(( $+functions[_geectl__subcmd__pull-request__subcmd__help__subcmd__comments_commands] )) ||
_geectl__subcmd__pull-request__subcmd__help__subcmd__comments_commands() {
    local commands; commands=()
    _describe -t commands 'geectl pull-request help comments commands' commands "$@"
}
(( $+functions[_geectl__subcmd__pull-request__subcmd__help__subcmd__decline_commands] )) ||
_geectl__subcmd__pull-request__subcmd__help__subcmd__decline_commands() {
    local commands; commands=()
    _describe -t commands 'geectl pull-request help decline commands' commands "$@"
}
(( $+functions[_geectl__subcmd__pull-request__subcmd__help__subcmd__diff_commands] )) ||
_geectl__subcmd__pull-request__subcmd__help__subcmd__diff_commands() {
    local commands; commands=()
    _describe -t commands 'geectl pull-request help diff commands' commands "$@"
}
(( $+functions[_geectl__subcmd__pull-request__subcmd__help__subcmd__help_commands] )) ||
_geectl__subcmd__pull-request__subcmd__help__subcmd__help_commands() {
    local commands; commands=()
    _describe -t commands 'geectl pull-request help help commands' commands "$@"
}
(( $+functions[_geectl__subcmd__pull-request__subcmd__help__subcmd__merge_commands] )) ||
_geectl__subcmd__pull-request__subcmd__help__subcmd__merge_commands() {
    local commands; commands=()
    _describe -t commands 'geectl pull-request help merge commands' commands "$@"
}
(( $+functions[_geectl__subcmd__pull-request__subcmd__help__subcmd__reopen_commands] )) ||
_geectl__subcmd__pull-request__subcmd__help__subcmd__reopen_commands() {
    local commands; commands=()
    _describe -t commands 'geectl pull-request help reopen commands' commands "$@"
}
(( $+functions[_geectl__subcmd__pull-request__subcmd__help__subcmd__request-changes_commands] )) ||
_geectl__subcmd__pull-request__subcmd__help__subcmd__request-changes_commands() {
    local commands; commands=()
    _describe -t commands 'geectl pull-request help request-changes commands' commands "$@"
}
(( $+functions[_geectl__subcmd__pull-request__subcmd__help__subcmd__search_commands] )) ||
_geectl__subcmd__pull-request__subcmd__help__subcmd__search_commands() {
    local commands; commands=()
    _describe -t commands 'geectl pull-request help search commands' commands "$@"
}
(( $+functions[_geectl__subcmd__pull-request__subcmd__help__subcmd__show_commands] )) ||
_geectl__subcmd__pull-request__subcmd__help__subcmd__show_commands() {
    local commands; commands=()
    _describe -t commands 'geectl pull-request help show commands' commands "$@"
}
(( $+functions[_geectl__subcmd__pull-request__subcmd__help__subcmd__submit_commands] )) ||
_geectl__subcmd__pull-request__subcmd__help__subcmd__submit_commands() {
    local commands; commands=()
    _describe -t commands 'geectl pull-request help submit commands' commands "$@"
}
(( $+functions[_geectl__subcmd__pull-request__subcmd__help__subcmd__withdraw_commands] )) ||
_geectl__subcmd__pull-request__subcmd__help__subcmd__withdraw_commands() {
    local commands; commands=()
    _describe -t commands 'geectl pull-request help withdraw commands' commands "$@"
}
(( $+functions[_geectl__subcmd__pull-request__subcmd__merge_commands] )) ||
_geectl__subcmd__pull-request__subcmd__merge_commands() {
    local commands; commands=()
    _describe -t commands 'geectl pull-request merge commands' commands "$@"
}
(( $+functions[_geectl__subcmd__pull-request__subcmd__reopen_commands] )) ||
_geectl__subcmd__pull-request__subcmd__reopen_commands() {
    local commands; commands=()
    _describe -t commands 'geectl pull-request reopen commands' commands "$@"
}
(( $+functions[_geectl__subcmd__pull-request__subcmd__request-changes_commands] )) ||
_geectl__subcmd__pull-request__subcmd__request-changes_commands() {
    local commands; commands=()
    _describe -t commands 'geectl pull-request request-changes commands' commands "$@"
}
(( $+functions[_geectl__subcmd__pull-request__subcmd__search_commands] )) ||
_geectl__subcmd__pull-request__subcmd__search_commands() {
    local commands; commands=()
    _describe -t commands 'geectl pull-request search commands' commands "$@"
}
(( $+functions[_geectl__subcmd__pull-request__subcmd__show_commands] )) ||
_geectl__subcmd__pull-request__subcmd__show_commands() {
    local commands; commands=()
    _describe -t commands 'geectl pull-request show commands' commands "$@"
}
(( $+functions[_geectl__subcmd__pull-request__subcmd__submit_commands] )) ||
_geectl__subcmd__pull-request__subcmd__submit_commands() {
    local commands; commands=()
    _describe -t commands 'geectl pull-request submit commands' commands "$@"
}
(( $+functions[_geectl__subcmd__pull-request__subcmd__withdraw_commands] )) ||
_geectl__subcmd__pull-request__subcmd__withdraw_commands() {
    local commands; commands=()
    _describe -t commands 'geectl pull-request withdraw commands' commands "$@"
}
(( $+functions[_geectl__subcmd__remote_commands] )) ||
_geectl__subcmd__remote_commands() {
    local commands; commands=(
'cat:Print a source file to standard output. Text only\: the library reads the file as a UTF-8 string, so a binary file fails the call with a server error ("response body is not valid UTF-8") instead of printing bytes' \
'download:Download a source file to a local path. The binary counterpart of \`cat\`\: the library streams the file to disk instead of reading it as a UTF-8 string, so a binary file (a source tarball) downloads instead of failing the UTF-8 check, and a file larger than the transport'\''s 10 MiB body cap downloads instead of failing. A read\: \`--live\` is ignored, like \`cat\`' \
'list-files:List the files of a package' \
'list-packages:List the packages of a project' \
'list-projects:List the top-level projects on the instance' \
'list-staging-projects:List the staging projects of a workflow project' \
'staging-project:Show one staging project of a workflow project' \
'package-log:Show the commit history of a package' \
'revision-patch:Show the per-file diff of one revision against the previous one. Computed client-side from per-file reads (the library lists both revisions and cats each changed file); a differing binary file fails the call' \
'project-meta:Show a project'\''s meta (title, description, persons, repositories)' \
'set-project-meta:Set a project'\''s meta, field by field or whole from a file. A mutation\: dry-run is the default (only the plan is printed); \`--live\` performs the write. Field flags change only the given fields; the rest of the meta is kept as it is. Clear a field with \`--from-file\`\: fields the file omits are cleared' \
'package-meta:Show a package'\''s meta (title, description, persons, devel project)' \
'set-package-meta:Set a package'\''s meta, field by field or whole from a file. A mutation\: dry-run is the default (only the plan is printed); \`--live\` performs the write. Field flags change only the given fields; the rest of the meta is kept as it is. Clear a field with \`--from-file\`\: fields the file omits are cleared' \
'update-package-meta-from-spec:Update a package'\''s meta from its spec file. A mutation\: dry-run is the default (only the plan is printed); \`--live\` performs the update. The spec file is found by listing the package files when exactly one \`*.spec\` exists; otherwise name it with \`--spec\`' \
'search-packages:Search packages by name. Substring matches on name, title, and description by default; \`--exact\` keeps only exact-name matches (the filter runs client-side). With \`--devel-project\`, only packages developed in that project; without a term, every package the devel project develops' \
'search-projects:Search projects by name. Substring matches on name, title, and description by default; \`--exact\` keeps only exact-name matches (the filter runs client-side)' \
'search-projects-by-person:Search projects by a person'\''s login\: every project whose meta names the login in a role' \
'search-packages-by-person:Search packages by a person'\''s login\: every package whose meta names the login in a role' \
'count-scmsync-projects:Count the projects syncing from a git URL prefix' \
'list-scmsync-projects:List the projects syncing from a git URL prefix' \
'owners:Show the owners of a package (project/package scope)' \
'owners-for-binary:Show the owners of a binary package' \
'owners-for-package:Show the owners of a package by name alone, in the owner root projects' \
'maintainers:Show the maintainers of a package' \
'whois:Show a user'\''s full name, email, and account state. Without a login, the authenticated user' \
'link-target:Show the link target of a package, when it is a link' \
'project-diff:Diff two projects, package by package. Computed client-side\: the library lists both projects and reads every file of every differing package, so a large diff issues many requests and a differing binary file fails the call' \
'diff-packages:Diff two packages, file by file. Computed client-side\: the library diffs the two file lists and cats each changed file, so a differing binary file fails the call' \
'devel-project:Show the devel project and devel package of a package, when it has one' \
'repo-add:Add a repository to a project. A mutation\: dry-run is the default (only the plan is printed); \`--live\` performs the addition' \
'repo-remove:Remove a repository from a project. A mutation\: dry-run is the default (only the plan is printed); \`--live\` performs the removal. Removing a repository that is not there succeeds' \
'upload-file:Upload a file into a package. A mutation\: dry-run is the default (only the plan is printed); \`--live\` performs the upload. The local file is read from \`--from\`; \`--binary\` uploads bytes instead of text' \
'create-project:Create a project. A mutation\: dry-run is the default (only the plan is printed); \`--live\` performs the creation' \
'create-package:Create a package. A mutation\: dry-run is the default (only the plan is printed); \`--live\` performs the creation. Without \`--scmsync\` and \`--force\` the package is created directly; with either, the creation goes through the options form, which refuses an existing package unless \`--force\` is set and refuses a project managed in git' \
'create-token:Create an operation-scoped trigger token for the authenticated user. A mutation\: dry-run is the default (only the plan is printed); \`--live\` performs the creation. The secret prints once, at creation\: store it now. Project and package go together\: give both to scope the token to a package, or neither for a global token' \
'delete-package:Delete a package. A mutation\: dry-run is the default (only the plan is printed); \`--live\` performs the deletion. Without \`--force\` a package with open submit requests is refused' \
'delete-link:Delete a \`_link\` file from a package. A mutation\: dry-run is the default (only the plan is printed); \`--live\` performs the deletion. The package keeps its files and meta, but stops tracking the source' \
'delete-file:Delete a file from a package. A mutation\: dry-run is the default (only the plan is printed); \`--live\` performs the deletion. The general form of \`delete-link\`\: deleting \`_link\` here issues the same delete as \`delete-link\`. Deleting a file that is already gone is refused by the server (NotFound). The server records the delete with the fixed comment \`removed <file>\`; there is no \`--message\` option, unlike \`delete-package\` — the asymmetry is intentional' \
'delete-project:Delete a project. A mutation\: dry-run is the default (only the plan is printed); \`--live\` performs the deletion. Without \`--recursive\` a project holding packages is refused' \
'undelete-package:Restore a deleted package. A mutation\: dry-run is the default (only the plan is printed); \`--live\` performs the restore' \
'undelete-project:Restore a deleted project. A mutation\: dry-run is the default (only the plan is printed); \`--live\` performs the restore' \
'aggregate-package:Aggregate a package'\''s binaries into another package. A mutation\: dry-run is the default (only the plan is printed); \`--live\` performs the aggregation. The target package is created from the source package'\''s meta when it does not exist yet; an \`_aggregate\` file that already exists is refused' \
'release-package:Release a package'\''s sources and binaries. A mutation\: dry-run is the default (only the plan is printed); \`--live\` performs the release. A release to a target project needs a repository and a target repository' \
'release-project:Release a whole project'\''s sources and binaries. A mutation\: dry-run is the default (only the plan is printed); \`--live\` performs the release. A release to a target project needs a repository and a target repository' \
'lock:Lock a project or a package. A mutation\: dry-run is the default (only the plan is printed); \`--live\` performs the lock. A project lock records the message on the lock; a package lock sends it but the server does not record it' \
'unlock:Unlock a project or a package. A mutation\: dry-run is the default (only the plan is printed); \`--live\` performs the unlock. The message is required\: the server refuses an unlock without a comment' \
'add-channels:Add the channels defined for a project or a package. A mutation\: dry-run is the default (only the plan is printed); \`--live\` performs the addition. A project or package with no channels defined is a no-op' \
'add-containers:Add the maintained containers for a package. A mutation\: dry-run is the default (only the plan is printed); \`--live\` performs the addition' \
'help:Print this message or the help of the given subcommand(s)' \
    )
    _describe -t commands 'geectl remote commands' commands "$@"
}
(( $+functions[_geectl__subcmd__remote__subcmd__add-channels_commands] )) ||
_geectl__subcmd__remote__subcmd__add-channels_commands() {
    local commands; commands=()
    _describe -t commands 'geectl remote add-channels commands' commands "$@"
}
(( $+functions[_geectl__subcmd__remote__subcmd__add-containers_commands] )) ||
_geectl__subcmd__remote__subcmd__add-containers_commands() {
    local commands; commands=()
    _describe -t commands 'geectl remote add-containers commands' commands "$@"
}
(( $+functions[_geectl__subcmd__remote__subcmd__aggregate-package_commands] )) ||
_geectl__subcmd__remote__subcmd__aggregate-package_commands() {
    local commands; commands=()
    _describe -t commands 'geectl remote aggregate-package commands' commands "$@"
}
(( $+functions[_geectl__subcmd__remote__subcmd__cat_commands] )) ||
_geectl__subcmd__remote__subcmd__cat_commands() {
    local commands; commands=()
    _describe -t commands 'geectl remote cat commands' commands "$@"
}
(( $+functions[_geectl__subcmd__remote__subcmd__count-scmsync-projects_commands] )) ||
_geectl__subcmd__remote__subcmd__count-scmsync-projects_commands() {
    local commands; commands=()
    _describe -t commands 'geectl remote count-scmsync-projects commands' commands "$@"
}
(( $+functions[_geectl__subcmd__remote__subcmd__create-package_commands] )) ||
_geectl__subcmd__remote__subcmd__create-package_commands() {
    local commands; commands=()
    _describe -t commands 'geectl remote create-package commands' commands "$@"
}
(( $+functions[_geectl__subcmd__remote__subcmd__create-project_commands] )) ||
_geectl__subcmd__remote__subcmd__create-project_commands() {
    local commands; commands=()
    _describe -t commands 'geectl remote create-project commands' commands "$@"
}
(( $+functions[_geectl__subcmd__remote__subcmd__create-token_commands] )) ||
_geectl__subcmd__remote__subcmd__create-token_commands() {
    local commands; commands=()
    _describe -t commands 'geectl remote create-token commands' commands "$@"
}
(( $+functions[_geectl__subcmd__remote__subcmd__delete-file_commands] )) ||
_geectl__subcmd__remote__subcmd__delete-file_commands() {
    local commands; commands=()
    _describe -t commands 'geectl remote delete-file commands' commands "$@"
}
(( $+functions[_geectl__subcmd__remote__subcmd__delete-link_commands] )) ||
_geectl__subcmd__remote__subcmd__delete-link_commands() {
    local commands; commands=()
    _describe -t commands 'geectl remote delete-link commands' commands "$@"
}
(( $+functions[_geectl__subcmd__remote__subcmd__delete-package_commands] )) ||
_geectl__subcmd__remote__subcmd__delete-package_commands() {
    local commands; commands=()
    _describe -t commands 'geectl remote delete-package commands' commands "$@"
}
(( $+functions[_geectl__subcmd__remote__subcmd__delete-project_commands] )) ||
_geectl__subcmd__remote__subcmd__delete-project_commands() {
    local commands; commands=()
    _describe -t commands 'geectl remote delete-project commands' commands "$@"
}
(( $+functions[_geectl__subcmd__remote__subcmd__devel-project_commands] )) ||
_geectl__subcmd__remote__subcmd__devel-project_commands() {
    local commands; commands=()
    _describe -t commands 'geectl remote devel-project commands' commands "$@"
}
(( $+functions[_geectl__subcmd__remote__subcmd__diff-packages_commands] )) ||
_geectl__subcmd__remote__subcmd__diff-packages_commands() {
    local commands; commands=()
    _describe -t commands 'geectl remote diff-packages commands' commands "$@"
}
(( $+functions[_geectl__subcmd__remote__subcmd__download_commands] )) ||
_geectl__subcmd__remote__subcmd__download_commands() {
    local commands; commands=()
    _describe -t commands 'geectl remote download commands' commands "$@"
}
(( $+functions[_geectl__subcmd__remote__subcmd__help_commands] )) ||
_geectl__subcmd__remote__subcmd__help_commands() {
    local commands; commands=(
'cat:Print a source file to standard output. Text only\: the library reads the file as a UTF-8 string, so a binary file fails the call with a server error ("response body is not valid UTF-8") instead of printing bytes' \
'download:Download a source file to a local path. The binary counterpart of \`cat\`\: the library streams the file to disk instead of reading it as a UTF-8 string, so a binary file (a source tarball) downloads instead of failing the UTF-8 check, and a file larger than the transport'\''s 10 MiB body cap downloads instead of failing. A read\: \`--live\` is ignored, like \`cat\`' \
'list-files:List the files of a package' \
'list-packages:List the packages of a project' \
'list-projects:List the top-level projects on the instance' \
'list-staging-projects:List the staging projects of a workflow project' \
'staging-project:Show one staging project of a workflow project' \
'package-log:Show the commit history of a package' \
'revision-patch:Show the per-file diff of one revision against the previous one. Computed client-side from per-file reads (the library lists both revisions and cats each changed file); a differing binary file fails the call' \
'project-meta:Show a project'\''s meta (title, description, persons, repositories)' \
'set-project-meta:Set a project'\''s meta, field by field or whole from a file. A mutation\: dry-run is the default (only the plan is printed); \`--live\` performs the write. Field flags change only the given fields; the rest of the meta is kept as it is. Clear a field with \`--from-file\`\: fields the file omits are cleared' \
'package-meta:Show a package'\''s meta (title, description, persons, devel project)' \
'set-package-meta:Set a package'\''s meta, field by field or whole from a file. A mutation\: dry-run is the default (only the plan is printed); \`--live\` performs the write. Field flags change only the given fields; the rest of the meta is kept as it is. Clear a field with \`--from-file\`\: fields the file omits are cleared' \
'update-package-meta-from-spec:Update a package'\''s meta from its spec file. A mutation\: dry-run is the default (only the plan is printed); \`--live\` performs the update. The spec file is found by listing the package files when exactly one \`*.spec\` exists; otherwise name it with \`--spec\`' \
'search-packages:Search packages by name. Substring matches on name, title, and description by default; \`--exact\` keeps only exact-name matches (the filter runs client-side). With \`--devel-project\`, only packages developed in that project; without a term, every package the devel project develops' \
'search-projects:Search projects by name. Substring matches on name, title, and description by default; \`--exact\` keeps only exact-name matches (the filter runs client-side)' \
'search-projects-by-person:Search projects by a person'\''s login\: every project whose meta names the login in a role' \
'search-packages-by-person:Search packages by a person'\''s login\: every package whose meta names the login in a role' \
'count-scmsync-projects:Count the projects syncing from a git URL prefix' \
'list-scmsync-projects:List the projects syncing from a git URL prefix' \
'owners:Show the owners of a package (project/package scope)' \
'owners-for-binary:Show the owners of a binary package' \
'owners-for-package:Show the owners of a package by name alone, in the owner root projects' \
'maintainers:Show the maintainers of a package' \
'whois:Show a user'\''s full name, email, and account state. Without a login, the authenticated user' \
'link-target:Show the link target of a package, when it is a link' \
'project-diff:Diff two projects, package by package. Computed client-side\: the library lists both projects and reads every file of every differing package, so a large diff issues many requests and a differing binary file fails the call' \
'diff-packages:Diff two packages, file by file. Computed client-side\: the library diffs the two file lists and cats each changed file, so a differing binary file fails the call' \
'devel-project:Show the devel project and devel package of a package, when it has one' \
'repo-add:Add a repository to a project. A mutation\: dry-run is the default (only the plan is printed); \`--live\` performs the addition' \
'repo-remove:Remove a repository from a project. A mutation\: dry-run is the default (only the plan is printed); \`--live\` performs the removal. Removing a repository that is not there succeeds' \
'upload-file:Upload a file into a package. A mutation\: dry-run is the default (only the plan is printed); \`--live\` performs the upload. The local file is read from \`--from\`; \`--binary\` uploads bytes instead of text' \
'create-project:Create a project. A mutation\: dry-run is the default (only the plan is printed); \`--live\` performs the creation' \
'create-package:Create a package. A mutation\: dry-run is the default (only the plan is printed); \`--live\` performs the creation. Without \`--scmsync\` and \`--force\` the package is created directly; with either, the creation goes through the options form, which refuses an existing package unless \`--force\` is set and refuses a project managed in git' \
'create-token:Create an operation-scoped trigger token for the authenticated user. A mutation\: dry-run is the default (only the plan is printed); \`--live\` performs the creation. The secret prints once, at creation\: store it now. Project and package go together\: give both to scope the token to a package, or neither for a global token' \
'delete-package:Delete a package. A mutation\: dry-run is the default (only the plan is printed); \`--live\` performs the deletion. Without \`--force\` a package with open submit requests is refused' \
'delete-link:Delete a \`_link\` file from a package. A mutation\: dry-run is the default (only the plan is printed); \`--live\` performs the deletion. The package keeps its files and meta, but stops tracking the source' \
'delete-file:Delete a file from a package. A mutation\: dry-run is the default (only the plan is printed); \`--live\` performs the deletion. The general form of \`delete-link\`\: deleting \`_link\` here issues the same delete as \`delete-link\`. Deleting a file that is already gone is refused by the server (NotFound). The server records the delete with the fixed comment \`removed <file>\`; there is no \`--message\` option, unlike \`delete-package\` — the asymmetry is intentional' \
'delete-project:Delete a project. A mutation\: dry-run is the default (only the plan is printed); \`--live\` performs the deletion. Without \`--recursive\` a project holding packages is refused' \
'undelete-package:Restore a deleted package. A mutation\: dry-run is the default (only the plan is printed); \`--live\` performs the restore' \
'undelete-project:Restore a deleted project. A mutation\: dry-run is the default (only the plan is printed); \`--live\` performs the restore' \
'aggregate-package:Aggregate a package'\''s binaries into another package. A mutation\: dry-run is the default (only the plan is printed); \`--live\` performs the aggregation. The target package is created from the source package'\''s meta when it does not exist yet; an \`_aggregate\` file that already exists is refused' \
'release-package:Release a package'\''s sources and binaries. A mutation\: dry-run is the default (only the plan is printed); \`--live\` performs the release. A release to a target project needs a repository and a target repository' \
'release-project:Release a whole project'\''s sources and binaries. A mutation\: dry-run is the default (only the plan is printed); \`--live\` performs the release. A release to a target project needs a repository and a target repository' \
'lock:Lock a project or a package. A mutation\: dry-run is the default (only the plan is printed); \`--live\` performs the lock. A project lock records the message on the lock; a package lock sends it but the server does not record it' \
'unlock:Unlock a project or a package. A mutation\: dry-run is the default (only the plan is printed); \`--live\` performs the unlock. The message is required\: the server refuses an unlock without a comment' \
'add-channels:Add the channels defined for a project or a package. A mutation\: dry-run is the default (only the plan is printed); \`--live\` performs the addition. A project or package with no channels defined is a no-op' \
'add-containers:Add the maintained containers for a package. A mutation\: dry-run is the default (only the plan is printed); \`--live\` performs the addition' \
'help:Print this message or the help of the given subcommand(s)' \
    )
    _describe -t commands 'geectl remote help commands' commands "$@"
}
(( $+functions[_geectl__subcmd__remote__subcmd__help__subcmd__add-channels_commands] )) ||
_geectl__subcmd__remote__subcmd__help__subcmd__add-channels_commands() {
    local commands; commands=()
    _describe -t commands 'geectl remote help add-channels commands' commands "$@"
}
(( $+functions[_geectl__subcmd__remote__subcmd__help__subcmd__add-containers_commands] )) ||
_geectl__subcmd__remote__subcmd__help__subcmd__add-containers_commands() {
    local commands; commands=()
    _describe -t commands 'geectl remote help add-containers commands' commands "$@"
}
(( $+functions[_geectl__subcmd__remote__subcmd__help__subcmd__aggregate-package_commands] )) ||
_geectl__subcmd__remote__subcmd__help__subcmd__aggregate-package_commands() {
    local commands; commands=()
    _describe -t commands 'geectl remote help aggregate-package commands' commands "$@"
}
(( $+functions[_geectl__subcmd__remote__subcmd__help__subcmd__cat_commands] )) ||
_geectl__subcmd__remote__subcmd__help__subcmd__cat_commands() {
    local commands; commands=()
    _describe -t commands 'geectl remote help cat commands' commands "$@"
}
(( $+functions[_geectl__subcmd__remote__subcmd__help__subcmd__count-scmsync-projects_commands] )) ||
_geectl__subcmd__remote__subcmd__help__subcmd__count-scmsync-projects_commands() {
    local commands; commands=()
    _describe -t commands 'geectl remote help count-scmsync-projects commands' commands "$@"
}
(( $+functions[_geectl__subcmd__remote__subcmd__help__subcmd__create-package_commands] )) ||
_geectl__subcmd__remote__subcmd__help__subcmd__create-package_commands() {
    local commands; commands=()
    _describe -t commands 'geectl remote help create-package commands' commands "$@"
}
(( $+functions[_geectl__subcmd__remote__subcmd__help__subcmd__create-project_commands] )) ||
_geectl__subcmd__remote__subcmd__help__subcmd__create-project_commands() {
    local commands; commands=()
    _describe -t commands 'geectl remote help create-project commands' commands "$@"
}
(( $+functions[_geectl__subcmd__remote__subcmd__help__subcmd__create-token_commands] )) ||
_geectl__subcmd__remote__subcmd__help__subcmd__create-token_commands() {
    local commands; commands=()
    _describe -t commands 'geectl remote help create-token commands' commands "$@"
}
(( $+functions[_geectl__subcmd__remote__subcmd__help__subcmd__delete-file_commands] )) ||
_geectl__subcmd__remote__subcmd__help__subcmd__delete-file_commands() {
    local commands; commands=()
    _describe -t commands 'geectl remote help delete-file commands' commands "$@"
}
(( $+functions[_geectl__subcmd__remote__subcmd__help__subcmd__delete-link_commands] )) ||
_geectl__subcmd__remote__subcmd__help__subcmd__delete-link_commands() {
    local commands; commands=()
    _describe -t commands 'geectl remote help delete-link commands' commands "$@"
}
(( $+functions[_geectl__subcmd__remote__subcmd__help__subcmd__delete-package_commands] )) ||
_geectl__subcmd__remote__subcmd__help__subcmd__delete-package_commands() {
    local commands; commands=()
    _describe -t commands 'geectl remote help delete-package commands' commands "$@"
}
(( $+functions[_geectl__subcmd__remote__subcmd__help__subcmd__delete-project_commands] )) ||
_geectl__subcmd__remote__subcmd__help__subcmd__delete-project_commands() {
    local commands; commands=()
    _describe -t commands 'geectl remote help delete-project commands' commands "$@"
}
(( $+functions[_geectl__subcmd__remote__subcmd__help__subcmd__devel-project_commands] )) ||
_geectl__subcmd__remote__subcmd__help__subcmd__devel-project_commands() {
    local commands; commands=()
    _describe -t commands 'geectl remote help devel-project commands' commands "$@"
}
(( $+functions[_geectl__subcmd__remote__subcmd__help__subcmd__diff-packages_commands] )) ||
_geectl__subcmd__remote__subcmd__help__subcmd__diff-packages_commands() {
    local commands; commands=()
    _describe -t commands 'geectl remote help diff-packages commands' commands "$@"
}
(( $+functions[_geectl__subcmd__remote__subcmd__help__subcmd__download_commands] )) ||
_geectl__subcmd__remote__subcmd__help__subcmd__download_commands() {
    local commands; commands=()
    _describe -t commands 'geectl remote help download commands' commands "$@"
}
(( $+functions[_geectl__subcmd__remote__subcmd__help__subcmd__help_commands] )) ||
_geectl__subcmd__remote__subcmd__help__subcmd__help_commands() {
    local commands; commands=()
    _describe -t commands 'geectl remote help help commands' commands "$@"
}
(( $+functions[_geectl__subcmd__remote__subcmd__help__subcmd__link-target_commands] )) ||
_geectl__subcmd__remote__subcmd__help__subcmd__link-target_commands() {
    local commands; commands=()
    _describe -t commands 'geectl remote help link-target commands' commands "$@"
}
(( $+functions[_geectl__subcmd__remote__subcmd__help__subcmd__list-files_commands] )) ||
_geectl__subcmd__remote__subcmd__help__subcmd__list-files_commands() {
    local commands; commands=()
    _describe -t commands 'geectl remote help list-files commands' commands "$@"
}
(( $+functions[_geectl__subcmd__remote__subcmd__help__subcmd__list-packages_commands] )) ||
_geectl__subcmd__remote__subcmd__help__subcmd__list-packages_commands() {
    local commands; commands=()
    _describe -t commands 'geectl remote help list-packages commands' commands "$@"
}
(( $+functions[_geectl__subcmd__remote__subcmd__help__subcmd__list-projects_commands] )) ||
_geectl__subcmd__remote__subcmd__help__subcmd__list-projects_commands() {
    local commands; commands=()
    _describe -t commands 'geectl remote help list-projects commands' commands "$@"
}
(( $+functions[_geectl__subcmd__remote__subcmd__help__subcmd__list-scmsync-projects_commands] )) ||
_geectl__subcmd__remote__subcmd__help__subcmd__list-scmsync-projects_commands() {
    local commands; commands=()
    _describe -t commands 'geectl remote help list-scmsync-projects commands' commands "$@"
}
(( $+functions[_geectl__subcmd__remote__subcmd__help__subcmd__list-staging-projects_commands] )) ||
_geectl__subcmd__remote__subcmd__help__subcmd__list-staging-projects_commands() {
    local commands; commands=()
    _describe -t commands 'geectl remote help list-staging-projects commands' commands "$@"
}
(( $+functions[_geectl__subcmd__remote__subcmd__help__subcmd__lock_commands] )) ||
_geectl__subcmd__remote__subcmd__help__subcmd__lock_commands() {
    local commands; commands=()
    _describe -t commands 'geectl remote help lock commands' commands "$@"
}
(( $+functions[_geectl__subcmd__remote__subcmd__help__subcmd__maintainers_commands] )) ||
_geectl__subcmd__remote__subcmd__help__subcmd__maintainers_commands() {
    local commands; commands=()
    _describe -t commands 'geectl remote help maintainers commands' commands "$@"
}
(( $+functions[_geectl__subcmd__remote__subcmd__help__subcmd__owners_commands] )) ||
_geectl__subcmd__remote__subcmd__help__subcmd__owners_commands() {
    local commands; commands=()
    _describe -t commands 'geectl remote help owners commands' commands "$@"
}
(( $+functions[_geectl__subcmd__remote__subcmd__help__subcmd__owners-for-binary_commands] )) ||
_geectl__subcmd__remote__subcmd__help__subcmd__owners-for-binary_commands() {
    local commands; commands=()
    _describe -t commands 'geectl remote help owners-for-binary commands' commands "$@"
}
(( $+functions[_geectl__subcmd__remote__subcmd__help__subcmd__owners-for-package_commands] )) ||
_geectl__subcmd__remote__subcmd__help__subcmd__owners-for-package_commands() {
    local commands; commands=()
    _describe -t commands 'geectl remote help owners-for-package commands' commands "$@"
}
(( $+functions[_geectl__subcmd__remote__subcmd__help__subcmd__package-log_commands] )) ||
_geectl__subcmd__remote__subcmd__help__subcmd__package-log_commands() {
    local commands; commands=()
    _describe -t commands 'geectl remote help package-log commands' commands "$@"
}
(( $+functions[_geectl__subcmd__remote__subcmd__help__subcmd__package-meta_commands] )) ||
_geectl__subcmd__remote__subcmd__help__subcmd__package-meta_commands() {
    local commands; commands=()
    _describe -t commands 'geectl remote help package-meta commands' commands "$@"
}
(( $+functions[_geectl__subcmd__remote__subcmd__help__subcmd__project-diff_commands] )) ||
_geectl__subcmd__remote__subcmd__help__subcmd__project-diff_commands() {
    local commands; commands=()
    _describe -t commands 'geectl remote help project-diff commands' commands "$@"
}
(( $+functions[_geectl__subcmd__remote__subcmd__help__subcmd__project-meta_commands] )) ||
_geectl__subcmd__remote__subcmd__help__subcmd__project-meta_commands() {
    local commands; commands=()
    _describe -t commands 'geectl remote help project-meta commands' commands "$@"
}
(( $+functions[_geectl__subcmd__remote__subcmd__help__subcmd__release-package_commands] )) ||
_geectl__subcmd__remote__subcmd__help__subcmd__release-package_commands() {
    local commands; commands=()
    _describe -t commands 'geectl remote help release-package commands' commands "$@"
}
(( $+functions[_geectl__subcmd__remote__subcmd__help__subcmd__release-project_commands] )) ||
_geectl__subcmd__remote__subcmd__help__subcmd__release-project_commands() {
    local commands; commands=()
    _describe -t commands 'geectl remote help release-project commands' commands "$@"
}
(( $+functions[_geectl__subcmd__remote__subcmd__help__subcmd__repo-add_commands] )) ||
_geectl__subcmd__remote__subcmd__help__subcmd__repo-add_commands() {
    local commands; commands=()
    _describe -t commands 'geectl remote help repo-add commands' commands "$@"
}
(( $+functions[_geectl__subcmd__remote__subcmd__help__subcmd__repo-remove_commands] )) ||
_geectl__subcmd__remote__subcmd__help__subcmd__repo-remove_commands() {
    local commands; commands=()
    _describe -t commands 'geectl remote help repo-remove commands' commands "$@"
}
(( $+functions[_geectl__subcmd__remote__subcmd__help__subcmd__revision-patch_commands] )) ||
_geectl__subcmd__remote__subcmd__help__subcmd__revision-patch_commands() {
    local commands; commands=()
    _describe -t commands 'geectl remote help revision-patch commands' commands "$@"
}
(( $+functions[_geectl__subcmd__remote__subcmd__help__subcmd__search-packages_commands] )) ||
_geectl__subcmd__remote__subcmd__help__subcmd__search-packages_commands() {
    local commands; commands=()
    _describe -t commands 'geectl remote help search-packages commands' commands "$@"
}
(( $+functions[_geectl__subcmd__remote__subcmd__help__subcmd__search-packages-by-person_commands] )) ||
_geectl__subcmd__remote__subcmd__help__subcmd__search-packages-by-person_commands() {
    local commands; commands=()
    _describe -t commands 'geectl remote help search-packages-by-person commands' commands "$@"
}
(( $+functions[_geectl__subcmd__remote__subcmd__help__subcmd__search-projects_commands] )) ||
_geectl__subcmd__remote__subcmd__help__subcmd__search-projects_commands() {
    local commands; commands=()
    _describe -t commands 'geectl remote help search-projects commands' commands "$@"
}
(( $+functions[_geectl__subcmd__remote__subcmd__help__subcmd__search-projects-by-person_commands] )) ||
_geectl__subcmd__remote__subcmd__help__subcmd__search-projects-by-person_commands() {
    local commands; commands=()
    _describe -t commands 'geectl remote help search-projects-by-person commands' commands "$@"
}
(( $+functions[_geectl__subcmd__remote__subcmd__help__subcmd__set-package-meta_commands] )) ||
_geectl__subcmd__remote__subcmd__help__subcmd__set-package-meta_commands() {
    local commands; commands=()
    _describe -t commands 'geectl remote help set-package-meta commands' commands "$@"
}
(( $+functions[_geectl__subcmd__remote__subcmd__help__subcmd__set-project-meta_commands] )) ||
_geectl__subcmd__remote__subcmd__help__subcmd__set-project-meta_commands() {
    local commands; commands=()
    _describe -t commands 'geectl remote help set-project-meta commands' commands "$@"
}
(( $+functions[_geectl__subcmd__remote__subcmd__help__subcmd__staging-project_commands] )) ||
_geectl__subcmd__remote__subcmd__help__subcmd__staging-project_commands() {
    local commands; commands=()
    _describe -t commands 'geectl remote help staging-project commands' commands "$@"
}
(( $+functions[_geectl__subcmd__remote__subcmd__help__subcmd__undelete-package_commands] )) ||
_geectl__subcmd__remote__subcmd__help__subcmd__undelete-package_commands() {
    local commands; commands=()
    _describe -t commands 'geectl remote help undelete-package commands' commands "$@"
}
(( $+functions[_geectl__subcmd__remote__subcmd__help__subcmd__undelete-project_commands] )) ||
_geectl__subcmd__remote__subcmd__help__subcmd__undelete-project_commands() {
    local commands; commands=()
    _describe -t commands 'geectl remote help undelete-project commands' commands "$@"
}
(( $+functions[_geectl__subcmd__remote__subcmd__help__subcmd__unlock_commands] )) ||
_geectl__subcmd__remote__subcmd__help__subcmd__unlock_commands() {
    local commands; commands=()
    _describe -t commands 'geectl remote help unlock commands' commands "$@"
}
(( $+functions[_geectl__subcmd__remote__subcmd__help__subcmd__update-package-meta-from-spec_commands] )) ||
_geectl__subcmd__remote__subcmd__help__subcmd__update-package-meta-from-spec_commands() {
    local commands; commands=()
    _describe -t commands 'geectl remote help update-package-meta-from-spec commands' commands "$@"
}
(( $+functions[_geectl__subcmd__remote__subcmd__help__subcmd__upload-file_commands] )) ||
_geectl__subcmd__remote__subcmd__help__subcmd__upload-file_commands() {
    local commands; commands=()
    _describe -t commands 'geectl remote help upload-file commands' commands "$@"
}
(( $+functions[_geectl__subcmd__remote__subcmd__help__subcmd__whois_commands] )) ||
_geectl__subcmd__remote__subcmd__help__subcmd__whois_commands() {
    local commands; commands=()
    _describe -t commands 'geectl remote help whois commands' commands "$@"
}
(( $+functions[_geectl__subcmd__remote__subcmd__link-target_commands] )) ||
_geectl__subcmd__remote__subcmd__link-target_commands() {
    local commands; commands=()
    _describe -t commands 'geectl remote link-target commands' commands "$@"
}
(( $+functions[_geectl__subcmd__remote__subcmd__list-files_commands] )) ||
_geectl__subcmd__remote__subcmd__list-files_commands() {
    local commands; commands=()
    _describe -t commands 'geectl remote list-files commands' commands "$@"
}
(( $+functions[_geectl__subcmd__remote__subcmd__list-packages_commands] )) ||
_geectl__subcmd__remote__subcmd__list-packages_commands() {
    local commands; commands=()
    _describe -t commands 'geectl remote list-packages commands' commands "$@"
}
(( $+functions[_geectl__subcmd__remote__subcmd__list-projects_commands] )) ||
_geectl__subcmd__remote__subcmd__list-projects_commands() {
    local commands; commands=()
    _describe -t commands 'geectl remote list-projects commands' commands "$@"
}
(( $+functions[_geectl__subcmd__remote__subcmd__list-scmsync-projects_commands] )) ||
_geectl__subcmd__remote__subcmd__list-scmsync-projects_commands() {
    local commands; commands=()
    _describe -t commands 'geectl remote list-scmsync-projects commands' commands "$@"
}
(( $+functions[_geectl__subcmd__remote__subcmd__list-staging-projects_commands] )) ||
_geectl__subcmd__remote__subcmd__list-staging-projects_commands() {
    local commands; commands=()
    _describe -t commands 'geectl remote list-staging-projects commands' commands "$@"
}
(( $+functions[_geectl__subcmd__remote__subcmd__lock_commands] )) ||
_geectl__subcmd__remote__subcmd__lock_commands() {
    local commands; commands=()
    _describe -t commands 'geectl remote lock commands' commands "$@"
}
(( $+functions[_geectl__subcmd__remote__subcmd__maintainers_commands] )) ||
_geectl__subcmd__remote__subcmd__maintainers_commands() {
    local commands; commands=()
    _describe -t commands 'geectl remote maintainers commands' commands "$@"
}
(( $+functions[_geectl__subcmd__remote__subcmd__owners_commands] )) ||
_geectl__subcmd__remote__subcmd__owners_commands() {
    local commands; commands=()
    _describe -t commands 'geectl remote owners commands' commands "$@"
}
(( $+functions[_geectl__subcmd__remote__subcmd__owners-for-binary_commands] )) ||
_geectl__subcmd__remote__subcmd__owners-for-binary_commands() {
    local commands; commands=()
    _describe -t commands 'geectl remote owners-for-binary commands' commands "$@"
}
(( $+functions[_geectl__subcmd__remote__subcmd__owners-for-package_commands] )) ||
_geectl__subcmd__remote__subcmd__owners-for-package_commands() {
    local commands; commands=()
    _describe -t commands 'geectl remote owners-for-package commands' commands "$@"
}
(( $+functions[_geectl__subcmd__remote__subcmd__package-log_commands] )) ||
_geectl__subcmd__remote__subcmd__package-log_commands() {
    local commands; commands=()
    _describe -t commands 'geectl remote package-log commands' commands "$@"
}
(( $+functions[_geectl__subcmd__remote__subcmd__package-meta_commands] )) ||
_geectl__subcmd__remote__subcmd__package-meta_commands() {
    local commands; commands=()
    _describe -t commands 'geectl remote package-meta commands' commands "$@"
}
(( $+functions[_geectl__subcmd__remote__subcmd__project-diff_commands] )) ||
_geectl__subcmd__remote__subcmd__project-diff_commands() {
    local commands; commands=()
    _describe -t commands 'geectl remote project-diff commands' commands "$@"
}
(( $+functions[_geectl__subcmd__remote__subcmd__project-meta_commands] )) ||
_geectl__subcmd__remote__subcmd__project-meta_commands() {
    local commands; commands=()
    _describe -t commands 'geectl remote project-meta commands' commands "$@"
}
(( $+functions[_geectl__subcmd__remote__subcmd__release-package_commands] )) ||
_geectl__subcmd__remote__subcmd__release-package_commands() {
    local commands; commands=()
    _describe -t commands 'geectl remote release-package commands' commands "$@"
}
(( $+functions[_geectl__subcmd__remote__subcmd__release-project_commands] )) ||
_geectl__subcmd__remote__subcmd__release-project_commands() {
    local commands; commands=()
    _describe -t commands 'geectl remote release-project commands' commands "$@"
}
(( $+functions[_geectl__subcmd__remote__subcmd__repo-add_commands] )) ||
_geectl__subcmd__remote__subcmd__repo-add_commands() {
    local commands; commands=()
    _describe -t commands 'geectl remote repo-add commands' commands "$@"
}
(( $+functions[_geectl__subcmd__remote__subcmd__repo-remove_commands] )) ||
_geectl__subcmd__remote__subcmd__repo-remove_commands() {
    local commands; commands=()
    _describe -t commands 'geectl remote repo-remove commands' commands "$@"
}
(( $+functions[_geectl__subcmd__remote__subcmd__revision-patch_commands] )) ||
_geectl__subcmd__remote__subcmd__revision-patch_commands() {
    local commands; commands=()
    _describe -t commands 'geectl remote revision-patch commands' commands "$@"
}
(( $+functions[_geectl__subcmd__remote__subcmd__search-packages_commands] )) ||
_geectl__subcmd__remote__subcmd__search-packages_commands() {
    local commands; commands=()
    _describe -t commands 'geectl remote search-packages commands' commands "$@"
}
(( $+functions[_geectl__subcmd__remote__subcmd__search-packages-by-person_commands] )) ||
_geectl__subcmd__remote__subcmd__search-packages-by-person_commands() {
    local commands; commands=()
    _describe -t commands 'geectl remote search-packages-by-person commands' commands "$@"
}
(( $+functions[_geectl__subcmd__remote__subcmd__search-projects_commands] )) ||
_geectl__subcmd__remote__subcmd__search-projects_commands() {
    local commands; commands=()
    _describe -t commands 'geectl remote search-projects commands' commands "$@"
}
(( $+functions[_geectl__subcmd__remote__subcmd__search-projects-by-person_commands] )) ||
_geectl__subcmd__remote__subcmd__search-projects-by-person_commands() {
    local commands; commands=()
    _describe -t commands 'geectl remote search-projects-by-person commands' commands "$@"
}
(( $+functions[_geectl__subcmd__remote__subcmd__set-package-meta_commands] )) ||
_geectl__subcmd__remote__subcmd__set-package-meta_commands() {
    local commands; commands=()
    _describe -t commands 'geectl remote set-package-meta commands' commands "$@"
}
(( $+functions[_geectl__subcmd__remote__subcmd__set-project-meta_commands] )) ||
_geectl__subcmd__remote__subcmd__set-project-meta_commands() {
    local commands; commands=()
    _describe -t commands 'geectl remote set-project-meta commands' commands "$@"
}
(( $+functions[_geectl__subcmd__remote__subcmd__staging-project_commands] )) ||
_geectl__subcmd__remote__subcmd__staging-project_commands() {
    local commands; commands=()
    _describe -t commands 'geectl remote staging-project commands' commands "$@"
}
(( $+functions[_geectl__subcmd__remote__subcmd__undelete-package_commands] )) ||
_geectl__subcmd__remote__subcmd__undelete-package_commands() {
    local commands; commands=()
    _describe -t commands 'geectl remote undelete-package commands' commands "$@"
}
(( $+functions[_geectl__subcmd__remote__subcmd__undelete-project_commands] )) ||
_geectl__subcmd__remote__subcmd__undelete-project_commands() {
    local commands; commands=()
    _describe -t commands 'geectl remote undelete-project commands' commands "$@"
}
(( $+functions[_geectl__subcmd__remote__subcmd__unlock_commands] )) ||
_geectl__subcmd__remote__subcmd__unlock_commands() {
    local commands; commands=()
    _describe -t commands 'geectl remote unlock commands' commands "$@"
}
(( $+functions[_geectl__subcmd__remote__subcmd__update-package-meta-from-spec_commands] )) ||
_geectl__subcmd__remote__subcmd__update-package-meta-from-spec_commands() {
    local commands; commands=()
    _describe -t commands 'geectl remote update-package-meta-from-spec commands' commands "$@"
}
(( $+functions[_geectl__subcmd__remote__subcmd__upload-file_commands] )) ||
_geectl__subcmd__remote__subcmd__upload-file_commands() {
    local commands; commands=()
    _describe -t commands 'geectl remote upload-file commands' commands "$@"
}
(( $+functions[_geectl__subcmd__remote__subcmd__whois_commands] )) ||
_geectl__subcmd__remote__subcmd__whois_commands() {
    local commands; commands=()
    _describe -t commands 'geectl remote whois commands' commands "$@"
}
(( $+functions[_geectl__subcmd__request_commands] )) ||
_geectl__subcmd__request_commands() {
    local commands; commands=(
'show:Show one submit request\: source/target/message, reviews, history' \
'log:Show one submit request'\''s history\: creation, then each state change' \
'search:List the submit requests matching the filters\: a typed search' \
'list:List submit requests\: the same search, with the states still awaiting action (new, review) as the default. A thin wrapper over the search operation\: the flags select the same server-side search' \
'diff:Show the diff of a submit request\: the unified diff, or the request XML with the diff injected with \`--xml\`' \
'list-comments:List the comments on a submit request, in id order' \
'accept:Accept a submit request, with a comment as the message left with the accept. An accepted request is forwarded to its link parent unless \`--no-forward\` is given; a refused forward is reported and the accepted request stands, with the work staying queued in the devel project. The forwarded request'\''s \`.changes\` entry is the original description, overridable with \`-m\` / \`--forward-message\`. Dry-run by default; mutates only with \`--live\`' \
'decline:Decline a submit request, with a comment as the reason' \
'revoke:Revoke (withdraw) a submit request, with a comment as the reason' \
'change-state:Change a submit request'\''s state to one of new/review/accepted/declined/revoked' \
'try-change-state:Best-effort state change\: try it, and report whether it happened' \
'add-review:Ask a user, group, project, or package to review a submit request' \
'change-review-state:Change the state of one review on a submit request\: accept it, decline it, or reopen it to new' \
'add-comment:Comment on a submit request, optionally as a reply to another comment' \
'file:File a submit request\: source package to a devel project, superseding every open submit request for the package from the same user' \
'refile:File a submit request and supersede an old one with it, plus every other open submit request for the package from the same user\: the new request is filed first, and only then is each open one superseded pointing at it' \
'supersede:Supersede one request with another existing one. A mutation that needs \`--live\`; the dry-runnable supersede flow is \`request refile --supersede\`' \
'file-add-role:File an add-role request\: grant a person or a group a role on a project or a package' \
'file-change-devel:File a change-devel request\: change a package'\''s devel project' \
'file-delete:File a delete request\: delete a package, a repository, or a whole project' \
'file-maintenance-incident:File a maintenance incident request\: ask the maintenance team to start a maintenance incident from a maintenance update' \
'refile-maintenance-incident:File a maintenance incident request and supersede an old one with it\: the new request is filed first, and only then is the old one superseded pointing at it' \
'help:Print this message or the help of the given subcommand(s)' \
    )
    _describe -t commands 'geectl request commands' commands "$@"
}
(( $+functions[_geectl__subcmd__request__subcmd__accept_commands] )) ||
_geectl__subcmd__request__subcmd__accept_commands() {
    local commands; commands=()
    _describe -t commands 'geectl request accept commands' commands "$@"
}
(( $+functions[_geectl__subcmd__request__subcmd__add-comment_commands] )) ||
_geectl__subcmd__request__subcmd__add-comment_commands() {
    local commands; commands=()
    _describe -t commands 'geectl request add-comment commands' commands "$@"
}
(( $+functions[_geectl__subcmd__request__subcmd__add-review_commands] )) ||
_geectl__subcmd__request__subcmd__add-review_commands() {
    local commands; commands=()
    _describe -t commands 'geectl request add-review commands' commands "$@"
}
(( $+functions[_geectl__subcmd__request__subcmd__change-review-state_commands] )) ||
_geectl__subcmd__request__subcmd__change-review-state_commands() {
    local commands; commands=()
    _describe -t commands 'geectl request change-review-state commands' commands "$@"
}
(( $+functions[_geectl__subcmd__request__subcmd__change-state_commands] )) ||
_geectl__subcmd__request__subcmd__change-state_commands() {
    local commands; commands=()
    _describe -t commands 'geectl request change-state commands' commands "$@"
}
(( $+functions[_geectl__subcmd__request__subcmd__decline_commands] )) ||
_geectl__subcmd__request__subcmd__decline_commands() {
    local commands; commands=()
    _describe -t commands 'geectl request decline commands' commands "$@"
}
(( $+functions[_geectl__subcmd__request__subcmd__diff_commands] )) ||
_geectl__subcmd__request__subcmd__diff_commands() {
    local commands; commands=()
    _describe -t commands 'geectl request diff commands' commands "$@"
}
(( $+functions[_geectl__subcmd__request__subcmd__file_commands] )) ||
_geectl__subcmd__request__subcmd__file_commands() {
    local commands; commands=()
    _describe -t commands 'geectl request file commands' commands "$@"
}
(( $+functions[_geectl__subcmd__request__subcmd__file-add-role_commands] )) ||
_geectl__subcmd__request__subcmd__file-add-role_commands() {
    local commands; commands=()
    _describe -t commands 'geectl request file-add-role commands' commands "$@"
}
(( $+functions[_geectl__subcmd__request__subcmd__file-change-devel_commands] )) ||
_geectl__subcmd__request__subcmd__file-change-devel_commands() {
    local commands; commands=()
    _describe -t commands 'geectl request file-change-devel commands' commands "$@"
}
(( $+functions[_geectl__subcmd__request__subcmd__file-delete_commands] )) ||
_geectl__subcmd__request__subcmd__file-delete_commands() {
    local commands; commands=()
    _describe -t commands 'geectl request file-delete commands' commands "$@"
}
(( $+functions[_geectl__subcmd__request__subcmd__file-maintenance-incident_commands] )) ||
_geectl__subcmd__request__subcmd__file-maintenance-incident_commands() {
    local commands; commands=()
    _describe -t commands 'geectl request file-maintenance-incident commands' commands "$@"
}
(( $+functions[_geectl__subcmd__request__subcmd__help_commands] )) ||
_geectl__subcmd__request__subcmd__help_commands() {
    local commands; commands=(
'show:Show one submit request\: source/target/message, reviews, history' \
'log:Show one submit request'\''s history\: creation, then each state change' \
'search:List the submit requests matching the filters\: a typed search' \
'list:List submit requests\: the same search, with the states still awaiting action (new, review) as the default. A thin wrapper over the search operation\: the flags select the same server-side search' \
'diff:Show the diff of a submit request\: the unified diff, or the request XML with the diff injected with \`--xml\`' \
'list-comments:List the comments on a submit request, in id order' \
'accept:Accept a submit request, with a comment as the message left with the accept. An accepted request is forwarded to its link parent unless \`--no-forward\` is given; a refused forward is reported and the accepted request stands, with the work staying queued in the devel project. The forwarded request'\''s \`.changes\` entry is the original description, overridable with \`-m\` / \`--forward-message\`. Dry-run by default; mutates only with \`--live\`' \
'decline:Decline a submit request, with a comment as the reason' \
'revoke:Revoke (withdraw) a submit request, with a comment as the reason' \
'change-state:Change a submit request'\''s state to one of new/review/accepted/declined/revoked' \
'try-change-state:Best-effort state change\: try it, and report whether it happened' \
'add-review:Ask a user, group, project, or package to review a submit request' \
'change-review-state:Change the state of one review on a submit request\: accept it, decline it, or reopen it to new' \
'add-comment:Comment on a submit request, optionally as a reply to another comment' \
'file:File a submit request\: source package to a devel project, superseding every open submit request for the package from the same user' \
'refile:File a submit request and supersede an old one with it, plus every other open submit request for the package from the same user\: the new request is filed first, and only then is each open one superseded pointing at it' \
'supersede:Supersede one request with another existing one. A mutation that needs \`--live\`; the dry-runnable supersede flow is \`request refile --supersede\`' \
'file-add-role:File an add-role request\: grant a person or a group a role on a project or a package' \
'file-change-devel:File a change-devel request\: change a package'\''s devel project' \
'file-delete:File a delete request\: delete a package, a repository, or a whole project' \
'file-maintenance-incident:File a maintenance incident request\: ask the maintenance team to start a maintenance incident from a maintenance update' \
'refile-maintenance-incident:File a maintenance incident request and supersede an old one with it\: the new request is filed first, and only then is the old one superseded pointing at it' \
'help:Print this message or the help of the given subcommand(s)' \
    )
    _describe -t commands 'geectl request help commands' commands "$@"
}
(( $+functions[_geectl__subcmd__request__subcmd__help__subcmd__accept_commands] )) ||
_geectl__subcmd__request__subcmd__help__subcmd__accept_commands() {
    local commands; commands=()
    _describe -t commands 'geectl request help accept commands' commands "$@"
}
(( $+functions[_geectl__subcmd__request__subcmd__help__subcmd__add-comment_commands] )) ||
_geectl__subcmd__request__subcmd__help__subcmd__add-comment_commands() {
    local commands; commands=()
    _describe -t commands 'geectl request help add-comment commands' commands "$@"
}
(( $+functions[_geectl__subcmd__request__subcmd__help__subcmd__add-review_commands] )) ||
_geectl__subcmd__request__subcmd__help__subcmd__add-review_commands() {
    local commands; commands=()
    _describe -t commands 'geectl request help add-review commands' commands "$@"
}
(( $+functions[_geectl__subcmd__request__subcmd__help__subcmd__change-review-state_commands] )) ||
_geectl__subcmd__request__subcmd__help__subcmd__change-review-state_commands() {
    local commands; commands=()
    _describe -t commands 'geectl request help change-review-state commands' commands "$@"
}
(( $+functions[_geectl__subcmd__request__subcmd__help__subcmd__change-state_commands] )) ||
_geectl__subcmd__request__subcmd__help__subcmd__change-state_commands() {
    local commands; commands=()
    _describe -t commands 'geectl request help change-state commands' commands "$@"
}
(( $+functions[_geectl__subcmd__request__subcmd__help__subcmd__decline_commands] )) ||
_geectl__subcmd__request__subcmd__help__subcmd__decline_commands() {
    local commands; commands=()
    _describe -t commands 'geectl request help decline commands' commands "$@"
}
(( $+functions[_geectl__subcmd__request__subcmd__help__subcmd__diff_commands] )) ||
_geectl__subcmd__request__subcmd__help__subcmd__diff_commands() {
    local commands; commands=()
    _describe -t commands 'geectl request help diff commands' commands "$@"
}
(( $+functions[_geectl__subcmd__request__subcmd__help__subcmd__file_commands] )) ||
_geectl__subcmd__request__subcmd__help__subcmd__file_commands() {
    local commands; commands=()
    _describe -t commands 'geectl request help file commands' commands "$@"
}
(( $+functions[_geectl__subcmd__request__subcmd__help__subcmd__file-add-role_commands] )) ||
_geectl__subcmd__request__subcmd__help__subcmd__file-add-role_commands() {
    local commands; commands=()
    _describe -t commands 'geectl request help file-add-role commands' commands "$@"
}
(( $+functions[_geectl__subcmd__request__subcmd__help__subcmd__file-change-devel_commands] )) ||
_geectl__subcmd__request__subcmd__help__subcmd__file-change-devel_commands() {
    local commands; commands=()
    _describe -t commands 'geectl request help file-change-devel commands' commands "$@"
}
(( $+functions[_geectl__subcmd__request__subcmd__help__subcmd__file-delete_commands] )) ||
_geectl__subcmd__request__subcmd__help__subcmd__file-delete_commands() {
    local commands; commands=()
    _describe -t commands 'geectl request help file-delete commands' commands "$@"
}
(( $+functions[_geectl__subcmd__request__subcmd__help__subcmd__file-maintenance-incident_commands] )) ||
_geectl__subcmd__request__subcmd__help__subcmd__file-maintenance-incident_commands() {
    local commands; commands=()
    _describe -t commands 'geectl request help file-maintenance-incident commands' commands "$@"
}
(( $+functions[_geectl__subcmd__request__subcmd__help__subcmd__help_commands] )) ||
_geectl__subcmd__request__subcmd__help__subcmd__help_commands() {
    local commands; commands=()
    _describe -t commands 'geectl request help help commands' commands "$@"
}
(( $+functions[_geectl__subcmd__request__subcmd__help__subcmd__list_commands] )) ||
_geectl__subcmd__request__subcmd__help__subcmd__list_commands() {
    local commands; commands=()
    _describe -t commands 'geectl request help list commands' commands "$@"
}
(( $+functions[_geectl__subcmd__request__subcmd__help__subcmd__list-comments_commands] )) ||
_geectl__subcmd__request__subcmd__help__subcmd__list-comments_commands() {
    local commands; commands=()
    _describe -t commands 'geectl request help list-comments commands' commands "$@"
}
(( $+functions[_geectl__subcmd__request__subcmd__help__subcmd__log_commands] )) ||
_geectl__subcmd__request__subcmd__help__subcmd__log_commands() {
    local commands; commands=()
    _describe -t commands 'geectl request help log commands' commands "$@"
}
(( $+functions[_geectl__subcmd__request__subcmd__help__subcmd__refile_commands] )) ||
_geectl__subcmd__request__subcmd__help__subcmd__refile_commands() {
    local commands; commands=()
    _describe -t commands 'geectl request help refile commands' commands "$@"
}
(( $+functions[_geectl__subcmd__request__subcmd__help__subcmd__refile-maintenance-incident_commands] )) ||
_geectl__subcmd__request__subcmd__help__subcmd__refile-maintenance-incident_commands() {
    local commands; commands=()
    _describe -t commands 'geectl request help refile-maintenance-incident commands' commands "$@"
}
(( $+functions[_geectl__subcmd__request__subcmd__help__subcmd__revoke_commands] )) ||
_geectl__subcmd__request__subcmd__help__subcmd__revoke_commands() {
    local commands; commands=()
    _describe -t commands 'geectl request help revoke commands' commands "$@"
}
(( $+functions[_geectl__subcmd__request__subcmd__help__subcmd__search_commands] )) ||
_geectl__subcmd__request__subcmd__help__subcmd__search_commands() {
    local commands; commands=()
    _describe -t commands 'geectl request help search commands' commands "$@"
}
(( $+functions[_geectl__subcmd__request__subcmd__help__subcmd__show_commands] )) ||
_geectl__subcmd__request__subcmd__help__subcmd__show_commands() {
    local commands; commands=()
    _describe -t commands 'geectl request help show commands' commands "$@"
}
(( $+functions[_geectl__subcmd__request__subcmd__help__subcmd__supersede_commands] )) ||
_geectl__subcmd__request__subcmd__help__subcmd__supersede_commands() {
    local commands; commands=()
    _describe -t commands 'geectl request help supersede commands' commands "$@"
}
(( $+functions[_geectl__subcmd__request__subcmd__help__subcmd__try-change-state_commands] )) ||
_geectl__subcmd__request__subcmd__help__subcmd__try-change-state_commands() {
    local commands; commands=()
    _describe -t commands 'geectl request help try-change-state commands' commands "$@"
}
(( $+functions[_geectl__subcmd__request__subcmd__list_commands] )) ||
_geectl__subcmd__request__subcmd__list_commands() {
    local commands; commands=()
    _describe -t commands 'geectl request list commands' commands "$@"
}
(( $+functions[_geectl__subcmd__request__subcmd__list-comments_commands] )) ||
_geectl__subcmd__request__subcmd__list-comments_commands() {
    local commands; commands=()
    _describe -t commands 'geectl request list-comments commands' commands "$@"
}
(( $+functions[_geectl__subcmd__request__subcmd__log_commands] )) ||
_geectl__subcmd__request__subcmd__log_commands() {
    local commands; commands=()
    _describe -t commands 'geectl request log commands' commands "$@"
}
(( $+functions[_geectl__subcmd__request__subcmd__refile_commands] )) ||
_geectl__subcmd__request__subcmd__refile_commands() {
    local commands; commands=()
    _describe -t commands 'geectl request refile commands' commands "$@"
}
(( $+functions[_geectl__subcmd__request__subcmd__refile-maintenance-incident_commands] )) ||
_geectl__subcmd__request__subcmd__refile-maintenance-incident_commands() {
    local commands; commands=()
    _describe -t commands 'geectl request refile-maintenance-incident commands' commands "$@"
}
(( $+functions[_geectl__subcmd__request__subcmd__revoke_commands] )) ||
_geectl__subcmd__request__subcmd__revoke_commands() {
    local commands; commands=()
    _describe -t commands 'geectl request revoke commands' commands "$@"
}
(( $+functions[_geectl__subcmd__request__subcmd__search_commands] )) ||
_geectl__subcmd__request__subcmd__search_commands() {
    local commands; commands=()
    _describe -t commands 'geectl request search commands' commands "$@"
}
(( $+functions[_geectl__subcmd__request__subcmd__show_commands] )) ||
_geectl__subcmd__request__subcmd__show_commands() {
    local commands; commands=()
    _describe -t commands 'geectl request show commands' commands "$@"
}
(( $+functions[_geectl__subcmd__request__subcmd__supersede_commands] )) ||
_geectl__subcmd__request__subcmd__supersede_commands() {
    local commands; commands=()
    _describe -t commands 'geectl request supersede commands' commands "$@"
}
(( $+functions[_geectl__subcmd__request__subcmd__try-change-state_commands] )) ||
_geectl__subcmd__request__subcmd__try-change-state_commands() {
    local commands; commands=()
    _describe -t commands 'geectl request try-change-state commands' commands "$@"
}
(( $+functions[_geectl__subcmd__resolve_commands] )) ||
_geectl__subcmd__resolve_commands() {
    local commands; commands=()
    _describe -t commands 'geectl resolve commands' commands "$@"
}
(( $+functions[_geectl__subcmd__rm_commands] )) ||
_geectl__subcmd__rm_commands() {
    local commands; commands=()
    _describe -t commands 'geectl rm commands' commands "$@"
}
(( $+functions[_geectl__subcmd__service_commands] )) ||
_geectl__subcmd__service_commands() {
    local commands; commands=(
'list:List the services defined in a package'\''s \`_service\` file' \
'state:Show the last service run state of a package' \
'trigger:Trigger a server-side run of a package'\''s services. A mutation\: dry-run is the default (only the plan is printed); \`--live\` performs the trigger. The trigger only queues the run\: the server runs the services asynchronously, so a successful trigger says the run was queued. Check the outcome with \`service state\`' \
'help:Print this message or the help of the given subcommand(s)' \
    )
    _describe -t commands 'geectl service commands' commands "$@"
}
(( $+functions[_geectl__subcmd__service__subcmd__help_commands] )) ||
_geectl__subcmd__service__subcmd__help_commands() {
    local commands; commands=(
'list:List the services defined in a package'\''s \`_service\` file' \
'state:Show the last service run state of a package' \
'trigger:Trigger a server-side run of a package'\''s services. A mutation\: dry-run is the default (only the plan is printed); \`--live\` performs the trigger. The trigger only queues the run\: the server runs the services asynchronously, so a successful trigger says the run was queued. Check the outcome with \`service state\`' \
'help:Print this message or the help of the given subcommand(s)' \
    )
    _describe -t commands 'geectl service help commands' commands "$@"
}
(( $+functions[_geectl__subcmd__service__subcmd__help__subcmd__help_commands] )) ||
_geectl__subcmd__service__subcmd__help__subcmd__help_commands() {
    local commands; commands=()
    _describe -t commands 'geectl service help help commands' commands "$@"
}
(( $+functions[_geectl__subcmd__service__subcmd__help__subcmd__list_commands] )) ||
_geectl__subcmd__service__subcmd__help__subcmd__list_commands() {
    local commands; commands=()
    _describe -t commands 'geectl service help list commands' commands "$@"
}
(( $+functions[_geectl__subcmd__service__subcmd__help__subcmd__state_commands] )) ||
_geectl__subcmd__service__subcmd__help__subcmd__state_commands() {
    local commands; commands=()
    _describe -t commands 'geectl service help state commands' commands "$@"
}
(( $+functions[_geectl__subcmd__service__subcmd__help__subcmd__trigger_commands] )) ||
_geectl__subcmd__service__subcmd__help__subcmd__trigger_commands() {
    local commands; commands=()
    _describe -t commands 'geectl service help trigger commands' commands "$@"
}
(( $+functions[_geectl__subcmd__service__subcmd__list_commands] )) ||
_geectl__subcmd__service__subcmd__list_commands() {
    local commands; commands=()
    _describe -t commands 'geectl service list commands' commands "$@"
}
(( $+functions[_geectl__subcmd__service__subcmd__state_commands] )) ||
_geectl__subcmd__service__subcmd__state_commands() {
    local commands; commands=()
    _describe -t commands 'geectl service state commands' commands "$@"
}
(( $+functions[_geectl__subcmd__service__subcmd__trigger_commands] )) ||
_geectl__subcmd__service__subcmd__trigger_commands() {
    local commands; commands=()
    _describe -t commands 'geectl service trigger commands' commands "$@"
}
(( $+functions[_geectl__subcmd__status_commands] )) ||
_geectl__subcmd__status_commands() {
    local commands; commands=()
    _describe -t commands 'geectl status commands' commands "$@"
}
(( $+functions[_geectl__subcmd__update_commands] )) ||
_geectl__subcmd__update_commands() {
    local commands; commands=()
    _describe -t commands 'geectl update commands' commands "$@"
}
(( $+functions[_geectl__subcmd__version_commands] )) ||
_geectl__subcmd__version_commands() {
    local commands; commands=()
    _describe -t commands 'geectl version commands' commands "$@"
}

if [ "$funcstack[1]" = "_geectl" ]; then
    _geectl "$@"
else
    compdef _geectl geectl
fi
