# ConnectAlso Daemon — Windows
#
# 选项 A: 使用 sc.exe 注册为 Windows 服务（需先安装为服务包装器）
#
#   sc.exe create ConnectAlsoDaemon \
#     binPath= "C:\Program Files\ConnectAlso\connectalso-daemon.exe --control-url http://127.0.0.1:3000 --stun-server 127.0.0.1:3478 --relay-server 127.0.0.1:33478 --hostname %COMPUTERNAME%" \
#     start= auto \
#     DisplayName= "ConnectAlso Daemon"
#
#   sc.exe start ConnectAlsoDaemon
#   sc.exe stop ConnectAlsoDaemon
#   sc.exe delete ConnectAlsoDaemon
#
# 选项 B: 使用 winsw (Windows Service Wrapper) — 推荐
#
#   1. 下载 winsw: https://github.com/winsw/winsw/releases
#   2. 将 WinSW-x64.exe 重命名为 connectalso-daemon-service.exe
#   3. 创建 connectalso-daemon-service.xml (见下方)
#   4. connectalso-daemon-service.exe install
#   5. connectalso-daemon-service.exe start
#
# 选项 C: 手动启动 (管理员 PowerShell)
#
#   Start-Process -FilePath "connectalso-daemon.exe" \
#     -ArgumentList "--control-url http://127.0.0.1:3000 --hostname $env:COMPUTERNAME" \
#     -WindowStyle Hidden
#
# Wintun 驱动:
#   从 https://www.wintun.net/ 下载 wintun.dll 放入
#   C:\Windows\System32\ 或 daemon 同级目录

# winsw XML config (connectalso-daemon-service.xml):
<service>
  <id>ConnectAlsoDaemon</id>
  <name>ConnectAlso Daemon</name>
  <description>ConnectAlso Virtual Network Daemon</description>
  <executable>connectalso-daemon.exe</executable>
  <arguments>--control-url http://127.0.0.1:3000 --stun-server 127.0.0.1:3478 --relay-server 127.0.0.1:33478</arguments>
  <log mode="roll-by-size">
    <sizeThreshold>10240</sizeThreshold>
    <keepFiles>3</keepFiles>
  </log>
  <onfailure action="restart" delay="5 sec"/>
</service>
