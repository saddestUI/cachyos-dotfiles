source /usr/share/cachyos-fish-config/cachyos-config.fish

# Keep fastfetch visible after clear and Ctrl+L
function clear --wraps=clear --description 'Clear screen and show fastfetch'
    command clear $argv
    if status is-interactive; and type -q fastfetch
        fastfetch
    end
end

function fish_user_key_bindings
    bind ctrl-l 'command clear; fastfetch; commandline -f repaint'
    if test "$fish_key_bindings" = fish_vi_key_bindings
        bind -M insert ctrl-l 'command clear; fastfetch; commandline -f repaint'
    end
end

# overwrite greeting
# potentially disabling fastfetch
#function fish_greeting
#    # smth smth
#end
