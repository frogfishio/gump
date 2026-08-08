[frogfish]
%{ for node in nodes ~}
${node.host_name} ansible_host=${node.public_ip} ansible_user=${ssh_user} cluster_private_ip=${node.private_ip}%{ if ssh_key_path != "" } ansible_ssh_private_key_file=${ssh_key_path}%{ endif }
%{ endfor ~}