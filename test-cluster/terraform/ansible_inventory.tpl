[gump]
%{ for node in nodes ~}
${node.host_name} ansible_host=${node.public_ip} ansible_user=${ssh_user} cluster_private_ip=${node.private_ip} gump_node_ordinal=${node.ordinal}%{ if ssh_key_path != "" } ansible_ssh_private_key_file=${ssh_key_path}%{ endif }
%{ endfor ~}

[gump_seed]
${nodes[0].host_name}

[gump_joiners]
%{ for node in slice(nodes, 1, length(nodes)) ~}
${node.host_name}
%{ endfor ~}
