# Print an optspec for argparse to handle cmd's options that are independent of any subcommand.
function __fish_cursor_session_global_optspecs
    string join \n storage= v/verbose color= h/help V/version
end

function __fish_cursor_session_needs_command
    # Figure out if the current invocation already has a command.
    set -l cmd (commandline -opc)
    set -e cmd[1]
    argparse -s (__fish_cursor_session_global_optspecs) -- $cmd 2>/dev/null
    or return
    if set -q argv[1]
        # Also print the command, so this can be used to figure out what it is.
        echo $argv[1]
        return 1
    end
    return 0
end

function __fish_cursor_session_using_subcommand
    set -l cmd (__fish_cursor_session_needs_command)
    test -z "$cmd"
    and return 1
    contains -- $cmd[1] $argv
end

complete -c cursor-session -n "__fish_cursor_session_needs_command" -l storage -d 'Read only this location: a home, .cursor, chats, workspace, session or projects directory, a store.db or state.vscdb file, or the directory that holds state.vscdb' -r -F
complete -c cursor-session -n "__fish_cursor_session_needs_command" -l color -d 'When to use color' -r -f -a "auto\t''
always\t''
never\t''"
complete -c cursor-session -n "__fish_cursor_session_needs_command" -s v -l verbose -d 'Print the storage paths in use and the rows and files that were skipped to stderr'
complete -c cursor-session -n "__fish_cursor_session_needs_command" -s h -l help -d 'Print help (see more with \'--help\')'
complete -c cursor-session -n "__fish_cursor_session_needs_command" -s V -l version -d 'Print version'
complete -c cursor-session -n "__fish_cursor_session_needs_command" -f -a "list" -d 'List sessions, most recently updated first'
complete -c cursor-session -n "__fish_cursor_session_needs_command" -f -a "show" -d 'Show messages from a session'
complete -c cursor-session -n "__fish_cursor_session_needs_command" -f -a "export" -d 'Export sessions to files'
complete -c cursor-session -n "__fish_cursor_session_needs_command" -f -a "healthcheck" -d 'Check that session stores can be found and loaded'
complete -c cursor-session -n "__fish_cursor_session_needs_command" -f -a "completions" -d 'Print a shell completion script'
complete -c cursor-session -n "__fish_cursor_session_needs_command" -f -a "man" -d 'Print a man page in roff format'
complete -c cursor-session -n "__fish_cursor_session_needs_command" -f -a "help" -d 'Print this message or the help of the given subcommand(s)'
complete -c cursor-session -n "__fish_cursor_session_using_subcommand list" -l source -d 'Only read this store; the other one is never opened' -r -f -a "agent\t'Cursor Agent CLI chats (~/.cursor/chats and agent transcripts)'
ide\t'Cursor IDE composer chats (state.vscdb)'"
complete -c cursor-session -n "__fish_cursor_session_using_subcommand list" -l limit -d 'Keep only the N most recently updated sessions' -r
complete -c cursor-session -n "__fish_cursor_session_using_subcommand list" -l storage -d 'Read only this location: a home, .cursor, chats, workspace, session or projects directory, a store.db or state.vscdb file, or the directory that holds state.vscdb' -r -F
complete -c cursor-session -n "__fish_cursor_session_using_subcommand list" -l color -d 'When to use color' -r -f -a "auto\t''
always\t''
never\t''"
complete -c cursor-session -n "__fish_cursor_session_using_subcommand list" -l json -d 'Print a JSON array of session summaries'
complete -c cursor-session -n "__fish_cursor_session_using_subcommand list" -s v -l verbose -d 'Print the storage paths in use and the rows and files that were skipped to stderr'
complete -c cursor-session -n "__fish_cursor_session_using_subcommand list" -s h -l help -d 'Print help (see more with \'--help\')'
complete -c cursor-session -n "__fish_cursor_session_using_subcommand show" -l source -d 'Only read this store; the other one is never opened' -r -f -a "agent\t'Cursor Agent CLI chats (~/.cursor/chats and agent transcripts)'
ide\t'Cursor IDE composer chats (state.vscdb)'"
complete -c cursor-session -n "__fish_cursor_session_using_subcommand show" -l limit -d 'Print only the last N messages [default: 20 in a terminal, all when piped or with --json]' -r
complete -c cursor-session -n "__fish_cursor_session_using_subcommand show" -l storage -d 'Read only this location: a home, .cursor, chats, workspace, session or projects directory, a store.db or state.vscdb file, or the directory that holds state.vscdb' -r -F
complete -c cursor-session -n "__fish_cursor_session_using_subcommand show" -l color -d 'When to use color' -r -f -a "auto\t''
always\t''
never\t''"
complete -c cursor-session -n "__fish_cursor_session_using_subcommand show" -l all -d 'Print the full transcript'
complete -c cursor-session -n "__fish_cursor_session_using_subcommand show" -l json -d 'Print the session and its messages as JSON'
complete -c cursor-session -n "__fish_cursor_session_using_subcommand show" -s v -l verbose -d 'Print the storage paths in use and the rows and files that were skipped to stderr'
complete -c cursor-session -n "__fish_cursor_session_using_subcommand show" -s h -l help -d 'Print help (see more with \'--help\')'
complete -c cursor-session -n "__fish_cursor_session_using_subcommand export" -l format -d 'Output file format' -r -f -a "md\t''
json\t''
jsonl\t''
yaml\t''"
complete -c cursor-session -n "__fish_cursor_session_using_subcommand export" -l out -d 'Directory to write into (created if missing)' -r -f -a "(__fish_complete_directories)"
complete -c cursor-session -n "__fish_cursor_session_using_subcommand export" -l session-id -d 'Export only this session (ID or unique prefix)' -r
complete -c cursor-session -n "__fish_cursor_session_using_subcommand export" -l workspace -d 'Export the Agent CLI sessions of a workspace: its path or a directory above it, directory names in its path, or the MD5 hash of its path' -r
complete -c cursor-session -n "__fish_cursor_session_using_subcommand export" -l source -d 'Only read this store; the other one is never opened' -r -f -a "agent\t'Cursor Agent CLI chats (~/.cursor/chats and agent transcripts)'
ide\t'Cursor IDE composer chats (state.vscdb)'"
complete -c cursor-session -n "__fish_cursor_session_using_subcommand export" -l limit -d 'Export only the N most recently updated of the selected sessions' -r
complete -c cursor-session -n "__fish_cursor_session_using_subcommand export" -l storage -d 'Read only this location: a home, .cursor, chats, workspace, session or projects directory, a store.db or state.vscdb file, or the directory that holds state.vscdb' -r -F
complete -c cursor-session -n "__fish_cursor_session_using_subcommand export" -l color -d 'When to use color' -r -f -a "auto\t''
always\t''
never\t''"
complete -c cursor-session -n "__fish_cursor_session_using_subcommand export" -s v -l verbose -d 'Print the storage paths in use and the rows and files that were skipped to stderr'
complete -c cursor-session -n "__fish_cursor_session_using_subcommand export" -s h -l help -d 'Print help (see more with \'--help\')'
complete -c cursor-session -n "__fish_cursor_session_using_subcommand healthcheck" -l storage -d 'Read only this location: a home, .cursor, chats, workspace, session or projects directory, a store.db or state.vscdb file, or the directory that holds state.vscdb' -r -F
complete -c cursor-session -n "__fish_cursor_session_using_subcommand healthcheck" -l color -d 'When to use color' -r -f -a "auto\t''
always\t''
never\t''"
complete -c cursor-session -n "__fish_cursor_session_using_subcommand healthcheck" -s v -l verbose -d 'Print the storage paths in use and the rows and files that were skipped to stderr'
complete -c cursor-session -n "__fish_cursor_session_using_subcommand healthcheck" -s h -l help -d 'Print help (see more with \'--help\')'
complete -c cursor-session -n "__fish_cursor_session_using_subcommand completions" -l storage -d 'Read only this location: a home, .cursor, chats, workspace, session or projects directory, a store.db or state.vscdb file, or the directory that holds state.vscdb' -r -F
complete -c cursor-session -n "__fish_cursor_session_using_subcommand completions" -l color -d 'When to use color' -r -f -a "auto\t''
always\t''
never\t''"
complete -c cursor-session -n "__fish_cursor_session_using_subcommand completions" -s v -l verbose -d 'Print the storage paths in use and the rows and files that were skipped to stderr'
complete -c cursor-session -n "__fish_cursor_session_using_subcommand completions" -s h -l help -d 'Print help (see more with \'--help\')'
complete -c cursor-session -n "__fish_cursor_session_using_subcommand man" -l storage -d 'Read only this location: a home, .cursor, chats, workspace, session or projects directory, a store.db or state.vscdb file, or the directory that holds state.vscdb' -r -F
complete -c cursor-session -n "__fish_cursor_session_using_subcommand man" -l color -d 'When to use color' -r -f -a "auto\t''
always\t''
never\t''"
complete -c cursor-session -n "__fish_cursor_session_using_subcommand man" -s v -l verbose -d 'Print the storage paths in use and the rows and files that were skipped to stderr'
complete -c cursor-session -n "__fish_cursor_session_using_subcommand man" -s h -l help -d 'Print help (see more with \'--help\')'
complete -c cursor-session -n "__fish_cursor_session_using_subcommand help; and not __fish_seen_subcommand_from list show export healthcheck completions man help" -f -a "list" -d 'List sessions, most recently updated first'
complete -c cursor-session -n "__fish_cursor_session_using_subcommand help; and not __fish_seen_subcommand_from list show export healthcheck completions man help" -f -a "show" -d 'Show messages from a session'
complete -c cursor-session -n "__fish_cursor_session_using_subcommand help; and not __fish_seen_subcommand_from list show export healthcheck completions man help" -f -a "export" -d 'Export sessions to files'
complete -c cursor-session -n "__fish_cursor_session_using_subcommand help; and not __fish_seen_subcommand_from list show export healthcheck completions man help" -f -a "healthcheck" -d 'Check that session stores can be found and loaded'
complete -c cursor-session -n "__fish_cursor_session_using_subcommand help; and not __fish_seen_subcommand_from list show export healthcheck completions man help" -f -a "completions" -d 'Print a shell completion script'
complete -c cursor-session -n "__fish_cursor_session_using_subcommand help; and not __fish_seen_subcommand_from list show export healthcheck completions man help" -f -a "man" -d 'Print a man page in roff format'
complete -c cursor-session -n "__fish_cursor_session_using_subcommand help; and not __fish_seen_subcommand_from list show export healthcheck completions man help" -f -a "help" -d 'Print this message or the help of the given subcommand(s)'
