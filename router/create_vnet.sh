# !/bin/bash

# set -eu pipefail

sudo ip netns add host1
sudo ip netns add router1
sudo ip netns add router2
sudo ip netns add host2

sudo ip link add name host1-router1 type veth peer name router1-host1
sudo ip link add name router1-router2 type veth peer name router2-router1
sudo ip link add name router2-host2 type veth peer name host2-router2

sudo ip link set host1-router1 netns host1
sudo ip link set router1-host1 netns router1
sudo ip link set router1-router2 netns router1
sudo ip link set router2-router1 netns router2
sudo ip link set router2-host2 netns router2
sudo ip link set host2-router2 netns host2

sudo ip netns exec host1 ip addr add 192.168.1.2/24 dev host1-router1
sudo ip netns exec host1 ip link set host1-router1 up
sudo ip netns exec host1 ethtool -K host1-router1 rx off tx off
sudo ip netns exec host1 ip route add default via 192.168.1.1

sudo ip netns exec router1 ip addr add 192.168.1.1/24 dev router1-host1
sudo ip netns exec router1 ip link set router1-host1 up
sudo ip netns exec router1 ethtool -K router1-host1 rx off tx off
sudo ip netns exec router1 ip addr add 192.168.0.1/24 dev router1-router2
sudo ip netns exec router1 ip link set router1-router2 up
sudo ip netns exec router1 ethtool -K router1-router2 rx off tx off
sudo ip netns exec router1 ip route add default via 192.168.0.2
sudo ip netns exec router1 sysctl -w net.ipv4.ip_forward=1

sudo ip netns exec router2 ip addr add 192.168.0.2/24 dev router2-router1
sudo ip netns exec router2 ip link set router2-router1 up
sudo ip netns exec router2 ethtool -K router2-router1 rx off tx off
sudo ip netns exec router2 ip route add default via 192.168.0.1
sudo ip netns exec router2 ip addr add 192.168.2.1/24 dev router2-host2
sudo ip netns exec router2 ip link set router2-host2 up
sudo ip netns exec router2 ethtool -K router2-host2 rx off tx off
sudo ip netns exec router2 sysctl -w net.ipv4.ip_forward=1

sudo ip netns exec host2 ip addr add 192.168.2.2/24 dev host2-router2
sudo ip netns exec host2 ip link set host2-router2 up
sudo ip netns exec host2 ethtool -K host2-router2 rx off tx off
sudo ip netns exec host2 ip route add default via 192.168.2.1
