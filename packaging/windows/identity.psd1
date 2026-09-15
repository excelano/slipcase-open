# The package identity Partner Center assigned, less the one value that is not
# this repository's to hold.
#
# `Publisher` is not here. It is the X.500 string Partner Center assigns per
# account, identical for every Excelano product, and it comes from the
# organisation secret STORE_PUBLISHER, which `windows.yml` passes to
# `build-msix.ps1` and which a Windows machine sets in its own environment.
# Everything below is public: the name and the store id are on the listing page
# and the package family name is in every package the Store distributes.
#
# Author: David M. Anderson
# Built with AI assistance (Claude, Anthropic)
@{
    # Package/Identity/Name
    Name = 'Excelano.SlipcaseOpen'

    # Package/Properties/PublisherDisplayName
    PublisherDisplayName = 'Excelano'

    # Calculated by Partner Center: the family name is what verifies an
    # install and what an AUMID is built from, the store id is what the
    # listing URL is built from.
    #
    #   Get-AppxPackage Excelano.SlipcaseOpen
    #   https://apps.microsoft.com/detail/9P5M97FB1HDW
    PackageFamilyName = 'Excelano.SlipcaseOpen_nbxmgv0sk86m4'
    StoreId = '9P5M97FB1HDW'
}
